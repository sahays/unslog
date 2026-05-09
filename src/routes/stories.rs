//! `/stories` — Story Builder. Pick a competency → AI-led probing chat →
//! Generate locks in a STAR+ bullet version. Refine = continue chat → vN+1.

use askama::Template;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use axum_extra::extract::Form;
use futures::TryStreamExt;
use mongodb::options::FindOptions;
use serde::Deserialize;

use std::collections::HashMap;

use crate::error::AppError;
use crate::models::{Category, ChatRole, ChatTurn, Story, StoryBody, StoryStatus, StoryVersion};
use crate::services::openrouter::{self, ChatMessage};
use crate::services::{category_store, prompt_store, settings_store};
use crate::startup::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/stories", get(index).post(create))
        .route("/stories/:id", get(show))
        .route("/stories/:id/turns", post(post_turn))
        .route("/stories/:id/generate", post(generate))
        .route("/stories/:id/continue", post(continue_chat))
        .route("/stories/:id/delete", post(delete_story))
        .route("/stories/:id/versions/:vid", get(show_version))
}

// ── Landing ──────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "stories/index.html")]
struct IndexTemplate {
    tiles: Vec<CompetencyTile>,
}

pub struct CompetencyTile {
    pub id: String,
    pub name: String,
    pub description: String,
    pub in_progress: usize,
    pub complete: usize,
    /// Most-recently-updated story id, if any. Drives whether the tile is a
    /// link (existing story) or a form-button (create first story).
    pub latest_story_id: Option<String>,
}

async fn index(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let categories = category_store::list_all(&state.db).await?;

    // Projected row shape — we don't need the embedded chat to render the
    // landing page. Match the projection exactly so deserialization
    // succeeds against the partial document.
    #[derive(serde::Deserialize)]
    struct StoryListRow {
        #[serde(rename = "_id")]
        id: String,
        competency_id: String,
        status: StoryStatus,
    }

    let opts = FindOptions::builder()
        .sort(bson::doc! { "updated_at": -1 })
        .projection(bson::doc! {
            "_id": 1,
            "competency_id": 1,
            "status": 1,
        })
        .build();
    let cursor = state
        .db
        .collection::<StoryListRow>(Story::COLLECTION)
        .find(bson::doc! {})
        .with_options(opts)
        .await?;
    let stories: Vec<StoryListRow> = cursor.try_collect().await?;

    let mut latest_by_comp: HashMap<String, String> = HashMap::new();
    let mut counts: HashMap<String, (usize, usize)> = HashMap::new();
    // The find query above sorts by updated_at desc — first row per
    // competency is the latest one, so we keep only the first id seen.
    for s in stories {
        let entry = counts.entry(s.competency_id.clone()).or_default();
        match s.status {
            StoryStatus::InProgress => entry.0 += 1,
            StoryStatus::Complete => entry.1 += 1,
        }
        latest_by_comp.entry(s.competency_id.clone()).or_insert(s.id);
    }

    let tiles: Vec<CompetencyTile> = categories
        .into_iter()
        .map(|c| {
            let (in_progress, complete) = counts.get(&c.id).copied().unwrap_or((0, 0));
            let latest_story_id = latest_by_comp.remove(&c.id);
            CompetencyTile {
                id: c.id,
                name: c.name,
                description: c.description,
                in_progress,
                complete,
                latest_story_id,
            }
        })
        .collect();

    let body = IndexTemplate { tiles }
        .render()
        .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(body))
}

// ── Create ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateForm {
    pub competency_id: String,
}

async fn create(
    State(state): State<AppState>,
    Form(form): Form<CreateForm>,
) -> Result<Response, AppError> {
    let competency_id = form.competency_id.trim().to_string();
    if competency_id.is_empty() {
        return Err(AppError::BadRequest("competency_id is required".into()));
    }
    let cat = category_store::get(&state.db, &competency_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("competency {competency_id}")))?;

    let now = chrono::Utc::now();
    let mut story = Story {
        id: uuid::Uuid::now_v7().to_string(),
        competency_id: cat.id.clone(),
        status: StoryStatus::InProgress,
        current_version_id: None,
        chat: Vec::new(),
        created_at: now,
        updated_at: now,
    };
    state
        .db
        .collection::<Story>(Story::COLLECTION)
        .insert_one(&story)
        .await?;
    tracing::info!(
        event = "story.create",
        story_id = %story.id,
        competency_id = %cat.id,
        "story created"
    );

    // Seed the opening probe so the candidate lands on a question, not a blank
    // chat. If the AI call fails, log and let the user kick it off by typing.
    if let Err(e) = open_chat(&state, &mut story, &cat).await {
        tracing::warn!(
            error = %e,
            story_id = %story.id,
            "failed to seed opening probe; user can kick off by typing",
        );
    }

    Ok(Redirect::to(&format!("/stories/{}", story.id)).into_response())
}

// ── Show ─────────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "stories/show.html")]
struct ShowTemplate {
    story: Story,
    competency: Category,
    current: Option<StoryVersion>,
    versions: Vec<VersionPickerEntry>,
    siblings: Vec<SiblingStory>,
}

pub struct VersionPickerEntry {
    pub id: String,
    pub label: String,
    pub is_current: bool,
}

pub struct SiblingStory {
    pub id: String,
    pub status: StoryStatus,
    pub updated_label: String,
}

async fn show(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Html<String>, AppError> {
    let story = load_story(&state, &id).await?;
    let competency = category_store::get(&state.db, &story.competency_id)
        .await?
        .unwrap_or_else(|| Category {
            id: story.competency_id.clone(),
            name: "Unknown competency".to_string(),
            description: String::new(),
            sort_order: 0,
            created_at: chrono::Utc::now(),
        });

    let current = match &story.current_version_id {
        Some(vid) => {
            state
                .db
                .collection::<StoryVersion>(StoryVersion::COLLECTION)
                .find_one(bson::doc! { "_id": vid })
                .await?
        }
        None => None,
    };

    let versions = list_versions_for_picker(&state, &story).await?;
    let siblings = list_sibling_stories(&state, &story).await?;

    let body = ShowTemplate {
        story,
        competency,
        current,
        versions,
        siblings,
    }
    .render()
    .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(body))
}

/// Other stories for the same competency, excluding the current one.
/// Sorted by most-recently-updated. Drives the side panel on the show page.
async fn list_sibling_stories(
    state: &AppState,
    story: &Story,
) -> Result<Vec<SiblingStory>, AppError> {
    #[derive(serde::Deserialize)]
    struct SiblingRow {
        #[serde(rename = "_id")]
        id: String,
        status: StoryStatus,
        #[serde(with = "crate::models::datetime_compat::required")]
        updated_at: chrono::DateTime<chrono::Utc>,
    }

    let opts = FindOptions::builder()
        .sort(bson::doc! { "updated_at": -1 })
        .projection(bson::doc! { "_id": 1, "status": 1, "updated_at": 1 })
        .build();
    let cursor = state
        .db
        .collection::<SiblingRow>(Story::COLLECTION)
        .find(bson::doc! {
            "competency_id": &story.competency_id,
            "_id": { "$ne": &story.id },
        })
        .with_options(opts)
        .await?;
    let rows: Vec<SiblingRow> = cursor.try_collect().await?;
    Ok(rows
        .into_iter()
        .map(|r| SiblingStory {
            id: r.id,
            status: r.status,
            updated_label: r.updated_at.format("%b %d, %H:%M").to_string(),
        })
        .collect())
}

async fn list_versions_for_picker(
    state: &AppState,
    story: &Story,
) -> Result<Vec<VersionPickerEntry>, AppError> {
    #[derive(serde::Deserialize)]
    struct VersionRow {
        #[serde(rename = "_id")]
        id: String,
        version_n: u32,
    }

    let opts = FindOptions::builder()
        .sort(bson::doc! { "version_n": 1 })
        .projection(bson::doc! { "_id": 1, "version_n": 1 })
        .build();
    let cursor = state
        .db
        .collection::<VersionRow>(StoryVersion::COLLECTION)
        .find(bson::doc! { "story_id": &story.id })
        .with_options(opts)
        .await?;
    let versions: Vec<VersionRow> = cursor.try_collect().await?;
    let current = story.current_version_id.as_deref();
    Ok(versions
        .into_iter()
        .map(|v| VersionPickerEntry {
            is_current: current == Some(v.id.as_str()),
            label: format!("v{}", v.version_n),
            id: v.id,
        })
        .collect())
}

// ── Post turn ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct TurnForm {
    pub content: String,
}

async fn post_turn(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Form(form): Form<TurnForm>,
) -> Result<Response, AppError> {
    let content = form.content.trim().to_string();
    if content.is_empty() {
        return Err(AppError::BadRequest("empty message".into()));
    }
    let mut story = load_story(&state, &id).await?;
    let competency = category_store::get(&state.db, &story.competency_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("competency {}", story.competency_id)))?;

    let user_turn = ChatTurn {
        role: ChatRole::User,
        content,
        ts: chrono::Utc::now(),
    };
    push_turn(&state, &mut story, user_turn).await?;

    // Ask the coach for its next reply. The AI itself decides whether the
    // candidate has agreed to lock in: if so, it ends its reply with the
    // literal `<<LOCK_IN>>` token (contract spelled out in
    // `prompts/story_chat.md`). We strip the token, persist the rest as the
    // final coach turn, and trigger run_generate.
    let raw = run_chat_model(&state, &story, &competency).await?;
    let (cleaned, lock_in) = strip_lock_in_token(&raw);

    if !cleaned.is_empty() {
        let assistant_turn = ChatTurn {
            role: ChatRole::Assistant,
            content: cleaned,
            ts: chrono::Utc::now(),
        };
        push_turn(&state, &mut story, assistant_turn).await?;
    }

    if lock_in {
        run_generate(&state, &story).await?;
    }

    Ok(Redirect::to(&format!("/stories/{id}")).into_response())
}

/// Sentinel token the coach emits when it judges the candidate has agreed
/// to lock in. Returns (cleaned_content, lock_in_detected). Cleaned content
/// has the token (and any whitespace immediately around it) removed.
const LOCK_IN_TOKEN: &str = "<<LOCK_IN>>";

fn strip_lock_in_token(s: &str) -> (String, bool) {
    if !s.contains(LOCK_IN_TOKEN) {
        return (s.trim().to_string(), false);
    }
    let cleaned = s.replace(LOCK_IN_TOKEN, "").trim().to_string();
    (cleaned, true)
}

// ── Generate ─────────────────────────────────────────────────────────────

async fn generate(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let story = load_story(&state, &id).await?;
    run_generate(&state, &story).await?;
    Ok(Redirect::to(&format!("/stories/{id}")).into_response())
}

/// Summarize the chat into a new StoryVersion, repoint `current_version_id`,
/// and flip status to `complete`. Shared by the explicit Generate route and
/// the agreement-driven flow inside `post_turn`.
async fn run_generate(state: &AppState, story: &Story) -> Result<StoryVersion, AppError> {
    if story.chat.is_empty() {
        return Err(AppError::BadRequest(
            "no chat content yet — answer at least one probe before generating".into(),
        ));
    }
    let settings = settings_store::load(&state.db).await?;
    let system = prompt_store::get_current_body(&state.db, "story_summarize").await?;

    let transcript = render_transcript(&story.chat);
    let user = format!("<chat_transcript>\n{transcript}\n</chat_transcript>");

    let raw = state
        .openrouter
        .chat(
            &settings.critique_model,
            vec![ChatMessage::system(system), ChatMessage::user(user)],
            true,
        )
        .await?;
    let body: StoryBody = openrouter::parse_json(&raw)?;

    let next_version_n = next_version_n(state, story).await?;
    let version = StoryVersion {
        id: uuid::Uuid::now_v7().to_string(),
        story_id: story.id.clone(),
        version_n: next_version_n,
        body,
        created_at: chrono::Utc::now(),
    };
    state
        .db
        .collection::<StoryVersion>(StoryVersion::COLLECTION)
        .insert_one(&version)
        .await?;
    state
        .db
        .collection::<Story>(Story::COLLECTION)
        .update_one(
            bson::doc! { "_id": &story.id },
            bson::doc! { "$set": {
                "current_version_id": &version.id,
                "status": "complete",
                "updated_at": chrono::Utc::now().to_rfc3339(),
            } },
        )
        .await?;
    tracing::info!(
        event = "story.generate",
        story_id = %story.id,
        version_id = %version.id,
        version_n = version.version_n,
        "story version generated"
    );
    Ok(version)
}

async fn next_version_n(state: &AppState, story: &Story) -> Result<u32, AppError> {
    #[derive(serde::Deserialize)]
    struct VersionNRow {
        version_n: u32,
    }

    let opts = FindOptions::builder()
        .sort(bson::doc! { "version_n": -1 })
        .limit(1)
        .projection(bson::doc! { "version_n": 1 })
        .build();
    let cursor = state
        .db
        .collection::<VersionNRow>(StoryVersion::COLLECTION)
        .find(bson::doc! { "story_id": &story.id })
        .with_options(opts)
        .await?;
    let latest: Vec<VersionNRow> = cursor.try_collect().await?;
    Ok(latest.first().map(|v| v.version_n + 1).unwrap_or(1))
}

fn render_transcript(chat: &[ChatTurn]) -> String {
    chat.iter()
        .map(|t| {
            let label = match t.role {
                ChatRole::User => "CANDIDATE",
                ChatRole::Assistant => "COACH",
            };
            format!("{label}:\n{}", t.content)
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

// ── Refine kickoff (Option X) ────────────────────────────────────────────

const REFINE_RECENT_CHAT_TURNS: usize = 8;

async fn continue_chat(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let mut story = load_story(&state, &id).await?;
    let Some(vid) = story.current_version_id.clone() else {
        return Err(AppError::BadRequest(
            "no current version to refine — generate v1 first".into(),
        ));
    };
    let version = state
        .db
        .collection::<StoryVersion>(StoryVersion::COLLECTION)
        .find_one(bson::doc! { "_id": &vid })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("story version {vid}")))?;

    let settings = settings_store::load(&state.db).await?;
    let system = prompt_store::get_current_body(&state.db, "story_refine_open").await?;

    let recent_slice: Vec<&ChatTurn> = story
        .chat
        .iter()
        .rev()
        .take(REFINE_RECENT_CHAT_TURNS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let recent_str = recent_slice
        .iter()
        .map(|t| {
            let label = match t.role {
                ChatRole::User => "CANDIDATE",
                ChatRole::Assistant => "COACH",
            };
            format!("{label}:\n{}", t.content)
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");

    let bullets = render_bullets(&version.body);
    let user = format!(
        "<current_version>\n{bullets}\n</current_version>\n\n<recent_chat>\n{recent_str}\n</recent_chat>"
    );

    let probe = state
        .openrouter
        .chat(
            &settings.critique_model,
            vec![ChatMessage::system(system), ChatMessage::user(user)],
            false,
        )
        .await?;
    let probe = probe.trim().to_string();

    let turn = ChatTurn {
        role: ChatRole::Assistant,
        content: probe,
        ts: chrono::Utc::now(),
    };
    push_turn(&state, &mut story, turn).await?;

    // Reopen the chat: status returns to InProgress so the next Generate
    // creates vN+1 instead of being a no-op visually.
    state
        .db
        .collection::<Story>(Story::COLLECTION)
        .update_one(
            bson::doc! { "_id": &story.id },
            bson::doc! { "$set": {
                "status": "in_progress",
                "updated_at": chrono::Utc::now().to_rfc3339(),
            } },
        )
        .await?;
    tracing::info!(
        event = "story.continue",
        story_id = %story.id,
        from_version = %vid,
        "refine kickoff probe appended"
    );

    Ok(Redirect::to(&format!("/stories/{id}")).into_response())
}

fn render_bullets(body: &StoryBody) -> String {
    fn section(label: &str, items: &[String]) -> String {
        if items.is_empty() {
            format!("{label}: (empty)\n")
        } else {
            let lines = items
                .iter()
                .map(|b| format!("- {b}"))
                .collect::<Vec<_>>()
                .join("\n");
            format!("{label}:\n{lines}\n")
        }
    }
    let mut out = String::new();
    out.push_str(&section("Situation", &body.situation));
    out.push('\n');
    out.push_str(&section("Task", &body.task));
    out.push('\n');
    out.push_str(&section("Action", &body.action));
    out.push('\n');
    out.push_str(&section("Result", &body.result));
    out.push('\n');
    out.push_str(&section("Reflection", &body.reflection));
    out
}

async fn delete_story(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    state
        .db
        .collection::<Story>(Story::COLLECTION)
        .delete_one(bson::doc! { "_id": &id })
        .await?;
    state
        .db
        .collection::<StoryVersion>(StoryVersion::COLLECTION)
        .delete_many(bson::doc! { "story_id": &id })
        .await?;
    tracing::info!(event = "story.delete", story_id = %id, "story cascade-deleted");
    Ok(Redirect::to("/stories").into_response())
}

// ── Past version (read-only) ─────────────────────────────────────────────

#[derive(Template)]
#[template(path = "stories/version.html")]
struct VersionTemplate {
    story: Story,
    competency: Category,
    version: StoryVersion,
    is_current: bool,
}

async fn show_version(
    State(state): State<AppState>,
    Path((id, vid)): Path<(String, String)>,
) -> Result<Html<String>, AppError> {
    let story = load_story(&state, &id).await?;
    let version = state
        .db
        .collection::<StoryVersion>(StoryVersion::COLLECTION)
        .find_one(bson::doc! { "_id": &vid, "story_id": &id })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("story version {vid}")))?;
    let competency = category_store::get(&state.db, &story.competency_id)
        .await?
        .unwrap_or_else(|| Category {
            id: story.competency_id.clone(),
            name: "Unknown competency".to_string(),
            description: String::new(),
            sort_order: 0,
            created_at: chrono::Utc::now(),
        });
    let is_current = story.current_version_id.as_deref() == Some(version.id.as_str());

    let body = VersionTemplate {
        story,
        competency,
        version,
        is_current,
    }
    .render()
    .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(body))
}

// ── Helpers ──────────────────────────────────────────────────────────────

async fn load_story(state: &AppState, id: &str) -> Result<Story, AppError> {
    state
        .db
        .collection::<Story>(Story::COLLECTION)
        .find_one(bson::doc! { "_id": id })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("story {id}")))
}

/// Append a chat turn and bump `updated_at`.
async fn push_turn(state: &AppState, story: &mut Story, turn: ChatTurn) -> Result<(), AppError> {
    let now = chrono::Utc::now();
    let turn_doc = bson::to_bson(&turn)?;
    state
        .db
        .collection::<Story>(Story::COLLECTION)
        .update_one(
            bson::doc! { "_id": &story.id },
            bson::doc! {
                "$push": { "chat": turn_doc },
                "$set":  { "updated_at": now.to_rfc3339() },
            },
        )
        .await?;
    story.chat.push(turn);
    story.updated_at = now;
    Ok(())
}

/// First-turn opener — call the model on a chat with no history. The system
/// prompt instructs it to ask one focused opening question.
async fn open_chat(
    state: &AppState,
    story: &mut Story,
    competency: &Category,
) -> Result<(), AppError> {
    let assistant = run_chat_model(state, story, competency).await?;
    let turn = ChatTurn {
        role: ChatRole::Assistant,
        content: assistant,
        ts: chrono::Utc::now(),
    };
    push_turn(state, story, turn).await?;
    Ok(())
}

/// Build [system, ...history] and call the chat model. Returns the model's
/// next assistant message content.
async fn run_chat_model(
    state: &AppState,
    story: &Story,
    competency: &Category,
) -> Result<String, AppError> {
    let settings = settings_store::load(&state.db).await?;
    let system_body = prompt_store::get_current_body(&state.db, "story_chat").await?;
    let competency_block = format!(
        "<competency>\nname: {}\nid: {}\ndescription: {}\n</competency>",
        competency.name, competency.id, competency.description
    );
    let system = format!("{system_body}\n\n{competency_block}");

    let mut messages: Vec<ChatMessage> = Vec::with_capacity(story.chat.len() + 2);
    messages.push(ChatMessage::system(system));
    for t in &story.chat {
        messages.push(ChatMessage {
            role: t.role.as_str().to_string(),
            content: t.content.clone(),
        });
    }
    // Most chat models won't generate from a system-only message. When the
    // chat is empty, prepend a one-line user nudge so the assistant produces
    // its opening probe.
    if story.chat.is_empty() {
        messages.push(ChatMessage::user(
            "Begin the conversation with one focused opening probe for this competency.",
        ));
    }

    let reply = state
        .openrouter
        .chat(&settings.critique_model, messages, false)
        .await?;
    Ok(reply.trim().to_string())
}

//! Chat-driven story flow:
//! * `post_turn` — the candidate sends a reply; the coach responds. If the
//!   coach signals lock-in via `<<LOCK_IN>>` we trigger `run_generate`.
//! * `generate` — explicit "lock it in now" button; calls `run_generate`.
//! * `continue_chat` — refine kickoff: append a fresh probe and reopen the
//!   chat for vN+1.

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::Form;
use futures::TryStreamExt;
use mongodb::options::FindOptions;
use serde::Deserialize;

use crate::error::AppError;
use crate::models::{ChatRole, ChatTurn, Story, StoryBody, StoryVersion};
use crate::services::openrouter::{self, ChatMessage};
use crate::services::{category_store, prompt_store, settings_store};
use crate::startup::AppState;

// ── Post turn ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(super) struct TurnForm {
    pub content: String,
}

pub(super) async fn post_turn(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Form(form): Form<TurnForm>,
) -> Result<Response, AppError> {
    let content = form.content.trim().to_string();
    if content.is_empty() {
        return Err(AppError::BadRequest("empty message".into()));
    }
    let mut story = super::load_story(&state, &id).await?;
    let competency = category_store::get(&state.db, &story.competency_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("competency {}", story.competency_id)))?;

    let user_turn = ChatTurn {
        role: ChatRole::User,
        content,
        ts: chrono::Utc::now(),
    };
    super::push_turn(&state, &mut story, user_turn).await?;

    // Ask the coach for its next reply. The AI itself decides whether the
    // candidate has agreed to lock in: if so, it ends its reply with the
    // literal `<<LOCK_IN>>` token (contract spelled out in
    // `prompts/story_chat.md`). We strip the token, persist the rest as the
    // final coach turn, and trigger run_generate.
    let raw = super::run_chat_model(&state, &story, &competency).await?;
    let (cleaned, lock_in) = strip_lock_in_token(&raw);

    if !cleaned.is_empty() {
        let assistant_turn = ChatTurn {
            role: ChatRole::Assistant,
            content: cleaned,
            ts: chrono::Utc::now(),
        };
        super::push_turn(&state, &mut story, assistant_turn).await?;
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

pub(super) async fn generate(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let story = super::load_story(&state, &id).await?;
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

// ── Refine kickoff ───────────────────────────────────────────────────────

const REFINE_RECENT_CHAT_TURNS: usize = 8;

pub(super) async fn continue_chat(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let mut story = super::load_story(&state, &id).await?;
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
    super::push_turn(&state, &mut story, turn).await?;

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

#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;

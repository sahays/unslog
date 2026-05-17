//! Stories landing page (`GET /stories`) and create (`POST /stories`).

use std::collections::HashMap;

use askama::Template;
use axum::extract::State;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum_extra::extract::Form;
use futures::TryStreamExt;
use mongodb::options::FindOptions;
use serde::Deserialize;

use crate::error::AppError;
use crate::filters; // Custom Askama filters used by templates below.
use crate::models::{Category, ChatRole, ChatTurn, Story, StoryStatus, StoryVersion};
use crate::services::category_store;
use crate::startup::AppState;

// ── Landing ──────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "stories/index.html")]
struct IndexTemplate {
    tiles: Vec<CompetencyTile>,
    completed_cards: Vec<CompletedCard>,
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

/// Compact card for a completed story's current version, rendered below
/// the tile grid. Links out to the version page (new tab) where the full
/// STAR+ bullets are shown — the card itself stays small.
pub struct CompletedCard {
    pub story_id: String,
    pub version_id: String,
    pub competency_name: String,
    pub version_label: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

pub(super) async fn index(State(state): State<AppState>) -> Result<Html<String>, AppError> {
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
        #[serde(default)]
        current_version_id: Option<String>,
        #[serde(with = "crate::models::datetime_compat::required")]
        updated_at: chrono::DateTime<chrono::Utc>,
    }

    let opts = FindOptions::builder()
        .sort(bson::doc! { "updated_at": -1 })
        .projection(bson::doc! {
            "_id": 1,
            "competency_id": 1,
            "status": 1,
            "current_version_id": 1,
            "updated_at": 1,
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
    for s in &stories {
        let entry = counts.entry(s.competency_id.clone()).or_default();
        match s.status {
            StoryStatus::InProgress => entry.0 += 1,
            StoryStatus::Complete => entry.1 += 1,
        }
        latest_by_comp
            .entry(s.competency_id.clone())
            .or_insert_with(|| s.id.clone());
    }

    // Bulk-load current StoryVersion docs for completed stories so we can
    // render bullet cards below the tile grid.
    let completed_version_ids: Vec<String> = stories
        .iter()
        .filter(|s| s.status == StoryStatus::Complete)
        .filter_map(|s| s.current_version_id.clone())
        .collect();

    // Project to just the fields needed for the card label — skip the
    // (potentially large) body, since we only render version_n + id and
    // link out to the version page on click.
    #[derive(serde::Deserialize)]
    struct VersionLabelRow {
        #[serde(rename = "_id")]
        id: String,
        version_n: u32,
    }

    let versions_by_id: HashMap<String, u32> = if completed_version_ids.is_empty() {
        HashMap::new()
    } else {
        let opts = FindOptions::builder()
            .projection(bson::doc! { "_id": 1, "version_n": 1 })
            .build();
        let cursor = state
            .db
            .collection::<VersionLabelRow>(StoryVersion::COLLECTION)
            .find(bson::doc! { "_id": { "$in": &completed_version_ids } })
            .with_options(opts)
            .await?;
        let rows: Vec<VersionLabelRow> = cursor.try_collect().await?;
        rows.into_iter().map(|v| (v.id, v.version_n)).collect()
    };

    let category_name_by_id: HashMap<String, String> = categories
        .iter()
        .map(|c| (c.id.clone(), c.name.clone()))
        .collect();

    // Sorted by updated_at desc (inherited from the stories cursor sort).
    let completed_cards: Vec<CompletedCard> = stories
        .iter()
        .filter(|s| s.status == StoryStatus::Complete)
        .filter_map(|s| {
            let vid = s.current_version_id.as_ref()?;
            let version_n = versions_by_id.get(vid)?;
            Some(CompletedCard {
                story_id: s.id.clone(),
                version_id: vid.clone(),
                competency_name: category_name_by_id
                    .get(&s.competency_id)
                    .cloned()
                    .unwrap_or_else(|| "Unknown competency".to_string()),
                version_label: format!("v{version_n}"),
                updated_at: s.updated_at,
            })
        })
        .collect();

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

    let body = IndexTemplate {
        tiles,
        completed_cards,
    }
    .render()
    .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(body))
}

// ── Create ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(super) struct CreateForm {
    pub competency_id: String,
}

pub(super) async fn create(
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
        mode: crate::models::Difficulty::default(),
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

/// First-turn opener — call the model on a chat with no history. The system
/// prompt instructs it to ask one focused opening question.
async fn open_chat(
    state: &AppState,
    story: &mut Story,
    competency: &Category,
) -> Result<(), AppError> {
    let assistant = super::run_chat_model(state, story, competency).await?;
    let turn = ChatTurn {
        role: ChatRole::Assistant,
        content: assistant,
        ts: chrono::Utc::now(),
    };
    super::push_turn(state, story, turn).await?;
    Ok(())
}

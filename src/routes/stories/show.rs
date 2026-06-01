//! Show one story (`GET /stories/:id`), one read-only past version
//! (`GET /stories/:id/versions/:vid`), and generate spoken variants for a
//! version (`POST /stories/:id/versions/:vid/spoken`).

use askama::Template;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};

use crate::error::AppError;
use crate::filters; // Custom Askama filters used by stories/show.html.
use crate::models::{Category, Story, StoryStatus, StoryVersion};
use crate::services::{category_store, story_spoken, story_store, story_version_store};
use crate::startup::AppState;

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
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

pub(super) async fn show(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Html<String>, AppError> {
    let story = super::load_story(&state, &id).await?;
    let competency = category_store::get(&state.pool, &story.competency_id)
        .await?
        .unwrap_or_else(|| super::unknown_competency(&story.competency_id));

    let current = match &story.current_version_id {
        Some(vid) => story_version_store::get(&state.pool, vid).await?,
        None => None,
    };

    let versions = picker_entries(&state, &story).await?;
    let siblings = siblings_view(&state, &story).await?;

    crate::error::render_html(ShowTemplate {
        story,
        competency,
        current,
        versions,
        siblings,
    })
}

async fn siblings_view(state: &AppState, story: &Story) -> Result<Vec<SiblingStory>, AppError> {
    let rows = story_store::list_siblings(&state.pool, &story.id, &story.competency_id).await?;
    Ok(rows
        .into_iter()
        .map(|r| SiblingStory {
            id: r.id,
            status: r.status,
            updated_at: r.updated_at,
        })
        .collect())
}

async fn picker_entries(
    state: &AppState,
    story: &Story,
) -> Result<Vec<VersionPickerEntry>, AppError> {
    let rows = story_version_store::list_for_picker(&state.pool, &story.id).await?;
    let current = story.current_version_id.as_deref();
    Ok(rows
        .into_iter()
        .map(|v| VersionPickerEntry {
            is_current: current == Some(v.id.as_str()),
            label: format!("v{}", v.version_n),
            id: v.id,
        })
        .collect())
}

#[derive(Template)]
#[template(path = "stories/version.html")]
struct VersionTemplate {
    story: Story,
    competency: Category,
    version: StoryVersion,
    is_current: bool,
}

pub(super) async fn show_version(
    State(state): State<AppState>,
    Path((id, vid)): Path<(String, String)>,
) -> Result<Html<String>, AppError> {
    let story = super::load_story(&state, &id).await?;
    let version = load_version(&state, &id, &vid).await?;
    let competency = category_store::get(&state.pool, &story.competency_id)
        .await?
        .unwrap_or_else(|| super::unknown_competency(&story.competency_id));
    let is_current = story.current_version_id.as_deref() == Some(version.id.as_str());

    crate::error::render_html(VersionTemplate {
        story,
        competency,
        version,
        is_current,
    })
}

/// Generate (or regenerate) the two spoken monologue variants for `vid` and
/// redirect back to the version page. Rerunning replaces whatever was there
/// — past spoken outputs are not retained because they're a derived view of
/// the bullets, not an independent artifact.
pub(super) async fn generate_spoken(
    State(state): State<AppState>,
    Path((id, vid)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let version = load_version(&state, &id, &vid).await?;
    story_spoken::generate_and_save(&state.pool, state.openrouter.as_ref(), &version).await?;
    Ok(Redirect::to(&format!("/stories/{id}/versions/{vid}")).into_response())
}

async fn load_version(
    state: &AppState,
    story_id: &str,
    version_id: &str,
) -> Result<StoryVersion, AppError> {
    story_version_store::find_by_story_and_id(&state.pool, story_id, version_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("story version {version_id}")))
}

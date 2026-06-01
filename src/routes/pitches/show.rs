//! Show one pitch (`GET /pitches/:slug`), one read-only past version
//! (`GET /pitches/:slug/versions/:vid`), and regenerate a version's
//! spoken prose (`POST /pitches/:slug/versions/:vid/regenerate`).

use askama::Template;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};

use crate::error::AppError;
use crate::filters; // Custom Askama filters used by pitches/*.html.
use crate::models::{Pitch, PitchStatus, PitchVersion};
use crate::services::{pitch_lockin, pitch_store, pitch_version_store};
use crate::startup::AppState;

#[derive(Template)]
#[template(path = "pitches/show.html")]
struct ShowTemplate {
    pitch: Pitch,
    current: Option<PitchVersion>,
    versions: Vec<VersionPickerEntry>,
    siblings: Vec<SiblingPitch>,
    /// Pre-formatted POST action for the chat composer. Built in the
    /// handler so the template doesn't have to dip into `format!()` inside
    /// a `{% call %}` argument list (Askama parses those, doesn't evaluate
    /// expression statements like `{% let %}` in every position).
    turns_action: String,
}

pub struct VersionPickerEntry {
    pub id: String,
    pub label: String,
    pub is_current: bool,
}

pub struct SiblingPitch {
    pub slug: String,
    pub question_text: String,
    pub status: PitchStatus,
}

pub(super) async fn show(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Html<String>, AppError> {
    let pitch = super::load_pitch(&state, &slug).await?;

    let current = match &pitch.current_version_id {
        Some(vid) => pitch_version_store::get(&state.db, vid).await?,
        None => None,
    };

    let versions = picker_entries(&state, &pitch).await?;
    let siblings = siblings_view(&state, &pitch).await?;
    let turns_action = format!("/pitches/{}/turns", pitch.id);

    crate::error::render_html(ShowTemplate {
        pitch,
        current,
        versions,
        siblings,
        turns_action,
    })
}

async fn siblings_view(state: &AppState, pitch: &Pitch) -> Result<Vec<SiblingPitch>, AppError> {
    let rows = pitch_store::list_siblings(&state.db, &pitch.id).await?;
    Ok(rows
        .into_iter()
        .map(|r| SiblingPitch {
            slug: r.slug,
            question_text: r.question_text,
            status: r.status,
        })
        .collect())
}

async fn picker_entries(
    state: &AppState,
    pitch: &Pitch,
) -> Result<Vec<VersionPickerEntry>, AppError> {
    let rows = pitch_version_store::list_for_picker(&state.db, &pitch.id).await?;
    let current = pitch.current_version_id.as_deref();
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
#[template(path = "pitches/version.html")]
struct VersionTemplate {
    pitch: Pitch,
    version: PitchVersion,
    is_current: bool,
}

pub(super) async fn show_version(
    State(state): State<AppState>,
    Path((slug, vid)): Path<(String, String)>,
) -> Result<Html<String>, AppError> {
    let pitch = super::load_pitch(&state, &slug).await?;
    let version = load_version(&state, &slug, &vid).await?;
    let is_current = pitch.current_version_id.as_deref() == Some(version.id.as_str());

    crate::error::render_html(VersionTemplate {
        pitch,
        version,
        is_current,
    })
}

/// Re-run the lock-in from the same chat to replace this version's
/// short+long in place. Past chat history is the source of truth; the user
/// hasn't changed it, but they may have edited the prompt or want a
/// different draft. Same handler used for both "regenerate" on the
/// current version and "regenerate" on a past version.
pub(super) async fn regenerate_version(
    State(state): State<AppState>,
    Path((slug, vid)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let pitch = super::load_pitch(&state, &slug).await?;
    let _existing = load_version(&state, &slug, &vid).await?;
    pitch_lockin::regenerate_version(
        &state.db,
        &state.pool,
        state.openrouter.as_ref(),
        &pitch,
        &vid,
    )
    .await?;
    Ok(Redirect::to(&format!("/pitches/{slug}/versions/{vid}")).into_response())
}

async fn load_version(
    state: &AppState,
    pitch_id: &str,
    version_id: &str,
) -> Result<PitchVersion, AppError> {
    pitch_version_store::find_by_pitch_and_id(&state.db, pitch_id, version_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("pitch version {version_id}")))
}

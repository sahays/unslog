//! Show one pitch (`GET /pitches/:slug`), one read-only past version
//! (`GET /pitches/:slug/versions/:vid`), and regenerate a version's
//! spoken prose (`POST /pitches/:slug/versions/:vid/regenerate`).

use askama::Template;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use futures::TryStreamExt;
use mongodb::options::FindOptions;

use crate::error::AppError;
use crate::filters; // Custom Askama filters used by pitches/*.html.
use crate::models::{Pitch, PitchStatus, PitchVersion};
use crate::services::pitch_lockin;
use crate::startup::AppState;

#[derive(Template)]
#[template(path = "pitches/show.html")]
struct ShowTemplate {
    pitch: Pitch,
    current: Option<PitchVersion>,
    versions: Vec<VersionPickerEntry>,
    siblings: Vec<SiblingPitch>,
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
        Some(vid) => {
            state
                .db
                .collection::<PitchVersion>(PitchVersion::COLLECTION)
                .find_one(bson::doc! { "_id": vid })
                .await?
        }
        None => None,
    };

    let versions = list_versions_for_picker(&state, &pitch).await?;
    let siblings = list_sibling_pitches(&state, &pitch).await?;

    let body = ShowTemplate {
        pitch,
        current,
        versions,
        siblings,
    }
    .render()
    .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(body))
}

/// Other pitches (everything except the current one), sorted by sort_order.
/// Drives the sidebar quick-jump panel on the show page.
async fn list_sibling_pitches(
    state: &AppState,
    pitch: &Pitch,
) -> Result<Vec<SiblingPitch>, AppError> {
    #[derive(serde::Deserialize)]
    struct SiblingRow {
        #[serde(rename = "_id")]
        id: String,
        question_text: String,
        status: PitchStatus,
    }

    let opts = FindOptions::builder()
        .sort(bson::doc! { "sort_order": 1 })
        .projection(bson::doc! { "_id": 1, "question_text": 1, "status": 1 })
        .build();
    let cursor = state
        .db
        .collection::<SiblingRow>(Pitch::COLLECTION)
        .find(bson::doc! { "_id": { "$ne": &pitch.id } })
        .with_options(opts)
        .await?;
    let rows: Vec<SiblingRow> = cursor.try_collect().await?;
    Ok(rows
        .into_iter()
        .map(|r| SiblingPitch {
            slug: r.id,
            question_text: r.question_text,
            status: r.status,
        })
        .collect())
}

async fn list_versions_for_picker(
    state: &AppState,
    pitch: &Pitch,
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
        .collection::<VersionRow>(PitchVersion::COLLECTION)
        .find(bson::doc! { "pitch_id": &pitch.id })
        .with_options(opts)
        .await?;
    let versions: Vec<VersionRow> = cursor.try_collect().await?;
    let current = pitch.current_version_id.as_deref();
    Ok(versions
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

    let body = VersionTemplate {
        pitch,
        version,
        is_current,
    }
    .render()
    .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(body))
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
    pitch_lockin::regenerate_version(&state.db, state.openrouter.as_ref(), &pitch, &vid).await?;
    Ok(Redirect::to(&format!("/pitches/{slug}/versions/{vid}")).into_response())
}

async fn load_version(
    state: &AppState,
    pitch_id: &str,
    version_id: &str,
) -> Result<PitchVersion, AppError> {
    state
        .db
        .collection::<PitchVersion>(PitchVersion::COLLECTION)
        .find_one(bson::doc! { "_id": version_id, "pitch_id": pitch_id })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("pitch version {version_id}")))
}

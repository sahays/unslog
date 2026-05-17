//! Show one story (`GET /stories/:id`) and one read-only past version
//! (`GET /stories/:id/versions/:vid`).

use askama::Template;
use axum::extract::{Path, State};
use axum::response::Html;
use futures::TryStreamExt;
use mongodb::options::FindOptions;

use crate::error::AppError;
use crate::filters; // Custom Askama filters used by stories/show.html.
use crate::models::{Category, Story, StoryStatus, StoryVersion};
use crate::services::category_store;
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
    let competency = category_store::get(&state.db, &story.competency_id)
        .await?
        .unwrap_or_else(|| super::unknown_competency(&story.competency_id));

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
            updated_at: r.updated_at,
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
    let version = state
        .db
        .collection::<StoryVersion>(StoryVersion::COLLECTION)
        .find_one(bson::doc! { "_id": &vid, "story_id": &id })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("story version {vid}")))?;
    let competency = category_store::get(&state.db, &story.competency_id)
        .await?
        .unwrap_or_else(|| super::unknown_competency(&story.competency_id));
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

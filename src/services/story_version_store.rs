//! Store for the `story_versions` collection — immutable StoryVersion docs.
//!
//! Read/write surface:
//! * `get` / `find_by_story_and_id` — lookups (parent-scoped variant for
//!   versions reached through `/stories/:id/versions/:vid` URLs).
//! * `insert_with_next_n` — atomic-ish "compute version_n + insert" with a
//!   single retry on the duplicate-key race (guarded by the unique
//!   compound index `(story_id, version_n)`).
//! * `list_for_picker` — projected sort for the version picker UI.
//! * `list_version_labels_by_ids` — bulk-load `_id → version_n` for the
//!   landing-page completed cards.
//! * `next_version_n` — monotonic counter per story (exposed for callers
//!   that need to know the next number without writing).
//! * `delete_for_story` — used by `story_store::delete_cascade`.

use std::collections::HashMap;

use futures::TryStreamExt;
use mongodb::options::FindOptions;
use mongodb::Database;

use crate::error::AppError;
use crate::models::StoryVersion;

pub struct VersionPickerRow {
    pub id: String,
    pub version_n: u32,
}

pub async fn get(db: &Database, version_id: &str) -> Result<Option<StoryVersion>, AppError> {
    Ok(db
        .collection::<StoryVersion>(StoryVersion::COLLECTION)
        .find_one(bson::doc! { "_id": version_id })
        .await?)
}

pub async fn find_by_story_and_id(
    db: &Database,
    story_id: &str,
    version_id: &str,
) -> Result<Option<StoryVersion>, AppError> {
    Ok(db
        .collection::<StoryVersion>(StoryVersion::COLLECTION)
        .find_one(bson::doc! { "_id": version_id, "story_id": story_id })
        .await?)
}

pub async fn insert(db: &Database, version: &StoryVersion) -> Result<(), AppError> {
    db.collection::<StoryVersion>(StoryVersion::COLLECTION)
        .insert_one(version)
        .await?;
    Ok(())
}

/// Compute `next_version_n` and insert a freshly-built version atomically
/// with respect to the unique `(story_id, version_n)` index. On a
/// duplicate-key collision (two locks-in racing on the same story), we
/// re-read `next_version_n` and retry the insert once. `build` runs
/// twice in that case — it must be deterministic in the
/// `version_n`-shaped fields (the caller usually closes over story_id
/// and body and only varies `version_n`).
pub async fn insert_with_next_n<F>(
    db: &Database,
    story_id: &str,
    build: F,
) -> Result<StoryVersion, AppError>
where
    F: Fn(u32) -> StoryVersion,
{
    let coll = db.collection::<StoryVersion>(StoryVersion::COLLECTION);
    let n = next_version_n(db, story_id).await?;
    let version = build(n);
    match coll.insert_one(&version).await {
        Ok(_) => Ok(version),
        Err(e) if crate::db::is_duplicate_key(&e) => {
            let n_retry = next_version_n(db, story_id).await?;
            let retry = build(n_retry);
            coll.insert_one(&retry).await?;
            Ok(retry)
        }
        Err(e) => Err(e.into()),
    }
}

pub async fn list_for_picker(
    db: &Database,
    story_id: &str,
) -> Result<Vec<VersionPickerRow>, AppError> {
    #[derive(serde::Deserialize)]
    struct Row {
        #[serde(rename = "_id")]
        id: String,
        version_n: u32,
    }
    let opts = FindOptions::builder()
        .sort(bson::doc! { "version_n": 1 })
        .projection(bson::doc! { "_id": 1, "version_n": 1 })
        .build();
    let cursor = db
        .collection::<Row>(StoryVersion::COLLECTION)
        .find(bson::doc! { "story_id": story_id })
        .with_options(opts)
        .await?;
    let rows: Vec<Row> = cursor.try_collect().await?;
    Ok(rows
        .into_iter()
        .map(|r| VersionPickerRow {
            id: r.id,
            version_n: r.version_n,
        })
        .collect())
}

/// Bulk lookup: map version_id → version_n for a set of ids. Used by the
/// landing page to render compact `vN` labels on completed cards in one
/// projected query instead of N per-card lookups.
pub async fn list_version_labels_by_ids(
    db: &Database,
    ids: &[String],
) -> Result<HashMap<String, u32>, AppError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    #[derive(serde::Deserialize)]
    struct VersionLabelRow {
        #[serde(rename = "_id")]
        id: String,
        version_n: u32,
    }
    let opts = FindOptions::builder()
        .projection(bson::doc! { "_id": 1, "version_n": 1 })
        .build();
    let cursor = db
        .collection::<VersionLabelRow>(StoryVersion::COLLECTION)
        .find(bson::doc! { "_id": { "$in": ids } })
        .with_options(opts)
        .await?;
    let rows: Vec<VersionLabelRow> = cursor.try_collect().await?;
    Ok(rows.into_iter().map(|v| (v.id, v.version_n)).collect())
}

/// Next monotonic `version_n` for a story (1-based). Returns 1 when no
/// versions exist yet.
pub async fn next_version_n(db: &Database, story_id: &str) -> Result<u32, AppError> {
    #[derive(serde::Deserialize)]
    struct VersionNRow {
        version_n: u32,
    }
    let opts = FindOptions::builder()
        .sort(bson::doc! { "version_n": -1 })
        .limit(1)
        .projection(bson::doc! { "version_n": 1 })
        .build();
    let cursor = db
        .collection::<VersionNRow>(StoryVersion::COLLECTION)
        .find(bson::doc! { "story_id": story_id })
        .with_options(opts)
        .await?;
    let latest: Vec<VersionNRow> = cursor.try_collect().await?;
    Ok(latest.first().map(|v| v.version_n + 1).unwrap_or(1))
}

/// Delete every version row for a story. Used by
/// `story_store::delete_cascade` — callers should not bypass it.
pub async fn delete_for_story(db: &Database, story_id: &str) -> Result<u64, AppError> {
    let res = db
        .collection::<StoryVersion>(StoryVersion::COLLECTION)
        .delete_many(bson::doc! { "story_id": story_id })
        .await?;
    Ok(res.deleted_count)
}

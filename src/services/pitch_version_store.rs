//! Store for the `pitch_versions` collection — immutable snapshots of
//! locked-in pitch answers. Owns reads/writes:
//!
//! * `find_by_pitch_and_id` — parent-scoped lookup (404-safe in callers).
//! * `list_for_picker` — projected sort for the version picker UI.
//! * `insert` — append a freshly built version.
//! * `next_version_n` — monotonic counter per pitch.
//! * `replace_in_place` — regenerate replaces short/long without bumping the
//!   version number (immutable id; mutable content).

use futures::TryStreamExt;
use mongodb::options::FindOptions;
use mongodb::Database;

use crate::error::AppError;
use crate::models::PitchVersion;

/// Projected row used by the version picker on `/pitches/:slug`. Carries
/// only what the template renders — no body, no timestamps.
pub struct VersionPickerRow {
    pub id: String,
    pub version_n: u32,
}

/// Parent-scoped lookup. The `pitch_id` filter prevents a malformed URL like
/// `/pitches/A/versions/<vid-from-pitch-B>` from leaking B's version into A's
/// page.
pub async fn find_by_pitch_and_id(
    db: &Database,
    pitch_id: &str,
    version_id: &str,
) -> Result<Option<PitchVersion>, AppError> {
    Ok(db
        .collection::<PitchVersion>(PitchVersion::COLLECTION)
        .find_one(bson::doc! { "_id": version_id, "pitch_id": pitch_id })
        .await?)
}

/// Lookup by version id alone. Used when the route already loaded the pitch
/// and knows the version belongs to it (e.g. show-page current-version
/// resolution where we just dereferenced `pitch.current_version_id`).
pub async fn get(db: &Database, version_id: &str) -> Result<Option<PitchVersion>, AppError> {
    Ok(db
        .collection::<PitchVersion>(PitchVersion::COLLECTION)
        .find_one(bson::doc! { "_id": version_id })
        .await?)
}

pub async fn insert(db: &Database, version: &PitchVersion) -> Result<(), AppError> {
    db.collection::<PitchVersion>(PitchVersion::COLLECTION)
        .insert_one(version)
        .await?;
    Ok(())
}

/// Compute `next_version_n` and insert a freshly-built version atomically
/// with respect to the unique `(pitch_id, version_n)` index. On a
/// duplicate-key collision (two locks-in racing on the same pitch), we
/// re-read `next_version_n` and retry the insert once. `build` runs
/// twice in that case — it must be deterministic in the
/// `version_n`-shaped fields.
pub async fn insert_with_next_n<F>(
    db: &Database,
    pitch_id: &str,
    build: F,
) -> Result<PitchVersion, AppError>
where
    F: Fn(u32) -> PitchVersion,
{
    let coll = db.collection::<PitchVersion>(PitchVersion::COLLECTION);
    let n = next_version_n(db, pitch_id).await?;
    let version = build(n);
    match coll.insert_one(&version).await {
        Ok(_) => Ok(version),
        Err(e) if crate::db::is_duplicate_key(&e) => {
            let n_retry = next_version_n(db, pitch_id).await?;
            let retry = build(n_retry);
            coll.insert_one(&retry).await?;
            Ok(retry)
        }
        Err(e) => Err(e.into()),
    }
}

/// All versions for a pitch in ascending order, projected for the picker.
pub async fn list_for_picker(
    db: &Database,
    pitch_id: &str,
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
        .collection::<Row>(PitchVersion::COLLECTION)
        .find(bson::doc! { "pitch_id": pitch_id })
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

/// Next monotonic `version_n` for a pitch (1-based). Returns 1 when no
/// versions exist yet.
pub async fn next_version_n(db: &Database, pitch_id: &str) -> Result<u32, AppError> {
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
        .collection::<VersionNRow>(PitchVersion::COLLECTION)
        .find(bson::doc! { "pitch_id": pitch_id })
        .with_options(opts)
        .await?;
    let latest: Vec<VersionNRow> = cursor.try_collect().await?;
    Ok(latest.first().map(|v| v.version_n + 1).unwrap_or(1))
}

/// Replace `short` and `long` on an existing version row in place. The id and
/// `version_n` stay the same — the regenerate flow's contract is "same
/// version slot, new content drawn from the same chat".
pub async fn replace_in_place(
    db: &Database,
    version_id: &str,
    pitch_id: &str,
    short: &str,
    long: &str,
) -> Result<(), AppError> {
    db.collection::<PitchVersion>(PitchVersion::COLLECTION)
        .update_one(
            bson::doc! { "_id": version_id, "pitch_id": pitch_id },
            bson::doc! { "$set": { "short": short, "long": long } },
        )
        .await?;
    Ok(())
}

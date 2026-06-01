//! Store for the Postgres `story_versions` table — immutable StoryVersion
//! rows. Every read filters by `owner_id`; every write sets it.
//!
//! Read/write surface:
//! * `get` / `find_by_story_and_id` — lookups (parent-scoped variant for
//!   versions reached through `/stories/:id/versions/:vid` URLs).
//! * `insert_with_next_n` — atomic "compute version_n + insert" with a
//!   single retry on the duplicate-key race (UNIQUE(story_id, version_n)).
//! * `set_spoken` — replace the spoken JSONB on a version.
//! * `list_for_picker` — projected sort for the version picker UI.
//! * `list_version_labels_by_ids` — bulk-load `id → version_n` for the
//!   landing-page completed cards.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::types::Json;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{SpokenStory, StoryBody, StoryVersion};
use crate::services::current_owner::TEMP_OWNER_ID;

/// Columns for full-row `StoryVersion` reads. Note the explicit JSONB-to-
/// `Json<...>` aliases so sqlx maps them straight onto `body` / `spoken`.
const VERSION_COLS: &str = r#"id, story_id, version_n,
    body AS "body: Json<StoryBody>",
    spoken AS "spoken: Json<SpokenStory>",
    created_at"#;

#[derive(sqlx::FromRow)]
struct StoryVersionRow {
    id: String,
    story_id: String,
    version_n: i32,
    body: Json<StoryBody>,
    spoken: Option<Json<SpokenStory>>,
    created_at: DateTime<Utc>,
}

impl StoryVersionRow {
    fn into_version(self) -> StoryVersion {
        StoryVersion {
            id: self.id,
            story_id: self.story_id,
            version_n: self.version_n as u32,
            body: self.body.0,
            spoken: self.spoken.map(|j| j.0),
            created_at: self.created_at,
        }
    }
}

/// Compact projection for the version picker dropdown on the show page.
pub struct VersionPickerRow {
    pub id: String,
    pub version_n: u32,
}

// ── Reads ────────────────────────────────────────────────────────────────

pub async fn get(pool: &PgPool, version_id: &str) -> Result<Option<StoryVersion>, AppError> {
    let sql = format!("SELECT {VERSION_COLS} FROM story_versions WHERE owner_id = $1 AND id = $2");
    let row: Option<StoryVersionRow> = sqlx::query_as(&sql)
        .bind(TEMP_OWNER_ID)
        .bind(version_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(StoryVersionRow::into_version))
}

pub async fn find_by_story_and_id(
    pool: &PgPool,
    story_id: &str,
    version_id: &str,
) -> Result<Option<StoryVersion>, AppError> {
    let sql = format!(
        "SELECT {VERSION_COLS} FROM story_versions \
         WHERE owner_id = $1 AND story_id = $2 AND id = $3"
    );
    let row: Option<StoryVersionRow> = sqlx::query_as(&sql)
        .bind(TEMP_OWNER_ID)
        .bind(story_id)
        .bind(version_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(StoryVersionRow::into_version))
}

pub async fn list_for_picker(
    pool: &PgPool,
    story_id: &str,
) -> Result<Vec<VersionPickerRow>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT id, version_n FROM story_versions
           WHERE owner_id = $1 AND story_id = $2
           ORDER BY version_n ASC"#,
        TEMP_OWNER_ID,
        story_id,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| VersionPickerRow {
            id: r.id,
            version_n: r.version_n as u32,
        })
        .collect())
}

/// Bulk lookup: map `id → version_n` for a set of version ids. Used by
/// the landing page to render compact `vN` labels on completed cards in
/// one query instead of N per-card lookups.
pub async fn list_version_labels_by_ids(
    pool: &PgPool,
    ids: &[String],
) -> Result<HashMap<String, u32>, AppError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query!(
        r#"SELECT id, version_n FROM story_versions
           WHERE owner_id = $1 AND id = ANY($2)"#,
        TEMP_OWNER_ID,
        ids,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.id, r.version_n as u32))
        .collect())
}

// ── Writes ───────────────────────────────────────────────────────────────

/// Compute the next `version_n` and insert a freshly-built version in one
/// CTE so the UNIQUE `(story_id, version_n)` constraint can never be racy
/// for a single caller. On a duplicate-key collision (two locks-in racing
/// on the same story), retry once.
pub async fn insert_with_next_n(
    pool: &PgPool,
    story_id: &str,
    body: &StoryBody,
    spoken: Option<&SpokenStory>,
) -> Result<StoryVersion, AppError> {
    match try_insert(pool, story_id, body, spoken).await {
        Ok(v) => Ok(v),
        Err(e) if is_dup_key(&e) => try_insert(pool, story_id, body, spoken).await,
        Err(e) => Err(e),
    }
}

/// True iff `e` is a `23505` unique-violation — the only retryable shape
/// for [`insert_with_next_n`]. Wraps the shared helper so the caller's
/// match is readable.
fn is_dup_key(e: &AppError) -> bool {
    if let AppError::Sqlx(err) = e {
        return crate::db::is_pg_duplicate_key(err);
    }
    false
}

/// Single attempt at the CTE-driven insert. Mints a fresh version id and
/// returns the resulting `StoryVersion`. The CTE reads `MAX(version_n)`
/// inside the same statement that inserts, so a serializable conflict
/// surfaces as a UNIQUE-violation we can retry once.
async fn try_insert(
    pool: &PgPool,
    story_id: &str,
    body: &StoryBody,
    spoken: Option<&SpokenStory>,
) -> Result<StoryVersion, AppError> {
    let id = StoryVersion::new_id();
    let row = sqlx::query!(
        r#"WITH next AS (
               SELECT COALESCE(MAX(version_n), 0) + 1 AS n
               FROM story_versions WHERE story_id = $1
           )
           INSERT INTO story_versions
               (id, story_id, owner_id, version_n, body, spoken, created_at)
           SELECT $2, $1, $3, next.n, $4, $5, NOW() FROM next
           RETURNING version_n, created_at"#,
        story_id,
        id,
        TEMP_OWNER_ID,
        serde_json::to_value(body).map_err(|e| AppError::Other(e.into()))?,
        spoken
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| AppError::Other(e.into()))?,
    )
    .fetch_one(pool)
    .await?;
    Ok(StoryVersion {
        id,
        story_id: story_id.to_string(),
        version_n: row.version_n as u32,
        body: body.clone(),
        spoken: spoken.cloned(),
        created_at: row.created_at,
    })
}

/// Replace the spoken JSONB on a version. Owner-scoped so callers don't
/// have to plumb the owner through. Used by `story_spoken::generate_and_save`.
pub async fn set_spoken(
    pool: &PgPool,
    version_id: &str,
    spoken: &SpokenStory,
) -> Result<(), AppError> {
    sqlx::query(
        r#"UPDATE story_versions
           SET spoken = $3
           WHERE owner_id = $1 AND id = $2"#,
    )
    .bind(TEMP_OWNER_ID)
    .bind(version_id)
    .bind(Json(spoken))
    .execute(pool)
    .await?;
    Ok(())
}

//! Store for the Postgres `pitch_versions` table — immutable snapshots of
//! locked-in pitch answers. Every read filters by `owner_id`; every write
//! sets it.
//!
//! Read/write surface:
//! * `get` / `find_by_pitch_and_id` — lookups (parent-scoped variant for
//!   versions reached through `/pitches/:slug/versions/:vid` URLs).
//! * `insert_with_next_n` — atomic "compute version_n + insert" with a
//!   single retry on the duplicate-key race
//!   (UNIQUE(pitch_id, version_n)).
//! * `list_for_picker` — projected sort for the version picker UI.
//! * `replace_in_place` — regenerate replaces short/long without bumping
//!   the version number (immutable id; mutable content).

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::PitchVersion;

/// Compact projection for the version picker dropdown on the show page.
pub struct VersionPickerRow {
    pub id: String,
    pub version_n: u32,
}

// ── Reads ────────────────────────────────────────────────────────────────

#[derive(sqlx::FromRow)]
struct PitchVersionRow {
    id: String,
    pitch_id: String,
    version_n: i32,
    short: String,
    long: String,
    created_at: DateTime<Utc>,
}

impl PitchVersionRow {
    fn into_version(self) -> PitchVersion {
        PitchVersion {
            id: self.id,
            pitch_id: self.pitch_id,
            version_n: self.version_n as u32,
            short: self.short,
            long: self.long,
            created_at: self.created_at,
        }
    }
}

const VERSION_COLS: &str = "id, pitch_id, version_n, short, long, created_at";

/// Lookup by version id alone. Owner-scoped. Used when the route already
/// loaded the pitch and knows the version belongs to it.
pub async fn get(
    pool: &PgPool,
    owner_id: &str,
    version_id: &str,
) -> Result<Option<PitchVersion>, AppError> {
    let sql = format!("SELECT {VERSION_COLS} FROM pitch_versions WHERE owner_id = $1 AND id = $2");
    let row: Option<PitchVersionRow> = sqlx::query_as(&sql)
        .bind(owner_id)
        .bind(version_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(PitchVersionRow::into_version))
}

/// Parent-scoped lookup. The `pitch_id` filter prevents a malformed URL
/// like `/pitches/A/versions/<vid-from-pitch-B>` from leaking B's version
/// into A's page.
pub async fn find_by_pitch_and_id(
    pool: &PgPool,
    owner_id: &str,
    pitch_id: &str,
    version_id: &str,
) -> Result<Option<PitchVersion>, AppError> {
    let sql = format!(
        "SELECT {VERSION_COLS} FROM pitch_versions \
         WHERE owner_id = $1 AND pitch_id = $2 AND id = $3"
    );
    let row: Option<PitchVersionRow> = sqlx::query_as(&sql)
        .bind(owner_id)
        .bind(pitch_id)
        .bind(version_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(PitchVersionRow::into_version))
}

/// All versions for a pitch in ascending order, projected for the picker.
pub async fn list_for_picker(
    pool: &PgPool,
    owner_id: &str,
    pitch_id: &str,
) -> Result<Vec<VersionPickerRow>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT id, version_n FROM pitch_versions
           WHERE owner_id = $1 AND pitch_id = $2
           ORDER BY version_n ASC"#,
        owner_id,
        pitch_id,
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

// ── Writes ───────────────────────────────────────────────────────────────

/// Compute the next `version_n` and insert a freshly-built version in one
/// CTE so the UNIQUE `(pitch_id, version_n)` constraint can never be racy
/// for a single caller. On a duplicate-key collision (two locks-in racing
/// on the same pitch), retry once.
pub async fn insert_with_next_n(
    pool: &PgPool,
    owner_id: &str,
    pitch_id: &str,
    short: &str,
    long: &str,
) -> Result<PitchVersion, AppError> {
    match try_insert(pool, owner_id, pitch_id, short, long).await {
        Ok(v) => Ok(v),
        Err(e) if is_dup_key(&e) => try_insert(pool, owner_id, pitch_id, short, long).await,
        Err(e) => Err(e),
    }
}

/// True iff `e` is a `23505` unique-violation — the only retryable shape
/// for [`insert_with_next_n`].
fn is_dup_key(e: &AppError) -> bool {
    if let AppError::Sqlx(err) = e {
        return crate::services::db::is_pg_duplicate_key(err);
    }
    false
}

/// Single attempt at the CTE-driven insert. Mints a fresh version id and
/// returns the resulting `PitchVersion`.
async fn try_insert(
    pool: &PgPool,
    owner_id: &str,
    pitch_id: &str,
    short: &str,
    long: &str,
) -> Result<PitchVersion, AppError> {
    let id = PitchVersion::new_id();
    let row = sqlx::query!(
        r#"WITH next AS (
               SELECT COALESCE(MAX(version_n), 0) + 1 AS n
               FROM pitch_versions WHERE pitch_id = $1
           )
           INSERT INTO pitch_versions
               (id, pitch_id, owner_id, version_n, short, long, created_at)
           SELECT $2, $1, $3, next.n, $4, $5, NOW() FROM next
           RETURNING version_n, created_at"#,
        pitch_id,
        id,
        owner_id,
        short,
        long,
    )
    .fetch_one(pool)
    .await?;
    Ok(PitchVersion {
        id,
        pitch_id: pitch_id.to_string(),
        version_n: row.version_n as u32,
        short: short.to_string(),
        long: long.to_string(),
        created_at: row.created_at,
    })
}

/// Replace `short` and `long` on an existing version row in place. The id
/// and `version_n` stay the same — the regenerate flow's contract is
/// "same version slot, new content drawn from the same chat".
pub async fn replace_in_place(
    pool: &PgPool,
    owner_id: &str,
    version_id: &str,
    pitch_id: &str,
    short: &str,
    long: &str,
) -> Result<(), AppError> {
    sqlx::query(
        r#"UPDATE pitch_versions
           SET short = $3, long = $4
           WHERE owner_id = $1 AND id = $2 AND pitch_id = $5"#,
    )
    .bind(owner_id)
    .bind(version_id)
    .bind(short)
    .bind(long)
    .bind(pitch_id)
    .execute(pool)
    .await?;
    Ok(())
}

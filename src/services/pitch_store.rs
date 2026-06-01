//! Canonical pitch catalog (`pitches`) + per-user state
//! (`pitch_user_state`). The catalog is seeded on first boot; per-user
//! state is created lazily by the first chat turn / lock-in.
//!
//! All reads do a LEFT JOIN so callers see a unified [`Pitch`] (catalog
//! cols + state cols, with `not_started` defaults when no state row
//! exists). All writes target `pitch_user_state` via UPSERT — the catalog
//! is read-only at runtime. Seed code lives in [`seed`] so this file stays
//! within the per-file LOC budget.

use chrono::{DateTime, Utc};
use sqlx::types::Json;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{ChatTurn, Pitch, PitchStatus};

#[path = "pitch_store_seed.rs"]
mod seed;
pub use seed::{pitch_question_id, seed_defaults};

/// Hard ceiling on the embedded chat array. The UX assumes a ~50-turn
/// working set; the cap blocks pathological loops well below the JSONB
/// document limit. Hitting the cap surfaces a 400 telling the user to
/// lock in and refine from there.
const MAX_CHAT_TURNS: usize = 100;

/// Pure capacity guard for [`push_turn`]. Split out for unit testing
/// without a live `&PgPool` handle.
fn check_chat_capacity(current_len: usize) -> Result<(), AppError> {
    if current_len >= MAX_CHAT_TURNS {
        return Err(AppError::BadRequest(format!(
            "chat is full ({MAX_CHAT_TURNS} turns) — lock this version in and refine from there"
        )));
    }
    Ok(())
}

// ── Row adapters ─────────────────────────────────────────────────────────

/// JOINed row used by `get` / `list_all`. The state-side columns are
/// NULL-safe via COALESCE in the SQL, so we can decode them as plain
/// non-Option fields here.
#[derive(sqlx::FromRow)]
struct PitchJoinRow {
    id: String,
    question_text: String,
    blurb: String,
    sort_order: i32,
    created_at: DateTime<Utc>,
    status: String,
    current_version_id: Option<String>,
    chat: Json<Vec<ChatTurn>>,
    updated_at: DateTime<Utc>,
}

impl PitchJoinRow {
    fn try_into_pitch(self) -> Result<Pitch, AppError> {
        Ok(Pitch {
            id: self.id,
            question_text: self.question_text,
            blurb: self.blurb,
            sort_order: self.sort_order,
            status: parse_status(&self.status)?,
            current_version_id: self.current_version_id,
            chat: self.chat.0,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

fn parse_status(s: &str) -> Result<PitchStatus, AppError> {
    PitchStatus::parse(s).ok_or_else(|| {
        AppError::Other(anyhow::anyhow!(
            "pitch_user_state.status {s:?} not in CHECK set"
        ))
    })
}

/// Columns selected by every catalog+state JOIN read. Centralized so a
/// schema add only edits one place. The SQL is interpolated into
/// `sqlx::query_as` (not the `query_as!` macro), so the chat alias is
/// plain — a `AS "name: Type"` annotation would become part of the
/// actual returned column name and break the `FromRow` lookup.
const JOIN_COLS: &str = r#"p.id, p.question_text, p.blurb, p.sort_order, p.created_at,
    COALESCE(s.status, 'not_started') AS status,
    s.current_version_id,
    COALESCE(s.chat, '[]'::jsonb) AS chat,
    COALESCE(s.updated_at, p.created_at) AS updated_at"#;

// ── Reads ────────────────────────────────────────────────────────────────

pub async fn get(pool: &PgPool, owner_id: &str, slug: &str) -> Result<Option<Pitch>, AppError> {
    let sql = format!(
        "SELECT {JOIN_COLS} FROM pitches p \
         LEFT JOIN pitch_user_state s \
         ON s.pitch_id = p.id AND s.owner_id = $1 \
         WHERE p.id = $2"
    );
    let row: Option<PitchJoinRow> = sqlx::query_as(&sql)
        .bind(owner_id)
        .bind(slug)
        .fetch_optional(pool)
        .await?;
    row.map(PitchJoinRow::try_into_pitch).transpose()
}

pub async fn list_all(pool: &PgPool, owner_id: &str) -> Result<Vec<Pitch>, AppError> {
    let sql = format!(
        "SELECT {JOIN_COLS} FROM pitches p \
         LEFT JOIN pitch_user_state s \
         ON s.pitch_id = p.id AND s.owner_id = $1 \
         ORDER BY p.sort_order"
    );
    let rows: Vec<PitchJoinRow> = sqlx::query_as(&sql).bind(owner_id).fetch_all(pool).await?;
    rows.into_iter().map(PitchJoinRow::try_into_pitch).collect()
}

/// Tight projection for the home page's "In progress" feed — never need
/// the embedded chat or version pointer to render the row.
pub struct InProgressRow {
    pub slug: String,
    pub question_text: String,
    pub updated_at: DateTime<Utc>,
}

/// In-progress pitches only, newest-touched-first. Inner-joined — only
/// pitches the user has touched count for this feed.
pub async fn list_in_progress(
    pool: &PgPool,
    owner_id: &str,
) -> Result<Vec<InProgressRow>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT p.id AS slug, p.question_text, s.updated_at
           FROM pitch_user_state s
           JOIN pitches p ON p.id = s.pitch_id
           WHERE s.owner_id = $1 AND s.status = 'in_progress'
           ORDER BY s.updated_at DESC"#,
        owner_id,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| InProgressRow {
            slug: r.slug,
            question_text: r.question_text,
            updated_at: r.updated_at,
        })
        .collect())
}

/// Sibling-pitch projection used by the show-page sidebar quick-jump panel.
/// Slim row — only what the template renders.
pub struct SiblingRow {
    pub slug: String,
    pub question_text: String,
    pub status: PitchStatus,
}

/// All other pitches (everything except `pitch_id`), sorted by `sort_order`,
/// each carrying its per-user status (or `not_started` when absent).
pub async fn list_siblings(
    pool: &PgPool,
    owner_id: &str,
    pitch_id: &str,
) -> Result<Vec<SiblingRow>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT p.id AS slug, p.question_text,
                  COALESCE(s.status, 'not_started') AS "status!"
           FROM pitches p
           LEFT JOIN pitch_user_state s
             ON s.pitch_id = p.id AND s.owner_id = $1
           WHERE p.id <> $2
           ORDER BY p.sort_order"#,
        owner_id,
        pitch_id,
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            Ok(SiblingRow {
                slug: r.slug,
                question_text: r.question_text,
                status: parse_status(&r.status)?,
            })
        })
        .collect()
}

// ── Writes (target `pitch_user_state`) ───────────────────────────────────

/// Append a chat turn and bump `updated_at`. UPSERTs the user-state row,
/// flipping `status` to `in_progress` (preserves `locked` to avoid
/// stomping a lock-in mid-refine). Refuses to grow `chat[]` past
/// [`MAX_CHAT_TURNS`].
pub async fn push_turn(
    pool: &PgPool,
    owner_id: &str,
    pitch: &mut Pitch,
    turn: ChatTurn,
) -> Result<(), AppError> {
    check_chat_capacity(pitch.chat.len())?;
    let now = chrono::Utc::now();
    let turn_json = serde_json::to_value(&turn).map_err(|e| AppError::Other(e.into()))?;
    sqlx::query(
        r#"INSERT INTO pitch_user_state
               (owner_id, pitch_id, status, chat, updated_at)
           VALUES ($1, $2, 'in_progress', jsonb_build_array($3::jsonb), $4)
           ON CONFLICT (owner_id, pitch_id) DO UPDATE
             SET chat = pitch_user_state.chat || $3::jsonb,
                 status = CASE WHEN pitch_user_state.status = 'locked'
                               THEN 'locked' ELSE 'in_progress' END,
                 updated_at = $4"#,
    )
    .bind(owner_id)
    .bind(&pitch.id)
    .bind(turn_json)
    .bind(now)
    .execute(pool)
    .await?;
    pitch.chat.push(turn);
    pitch.updated_at = now;
    if matches!(pitch.status, PitchStatus::NotStarted) {
        pitch.status = PitchStatus::InProgress;
    }
    Ok(())
}

/// Update `current_version_id` and flip status to Locked. UPSERTs the
/// state row so a lock-in can land on a pitch that has no state row yet
/// (defensive — `push_turn` will have created one in normal flow).
pub async fn set_current_version(
    pool: &PgPool,
    owner_id: &str,
    pitch_id: &str,
    version_id: &str,
) -> Result<(), AppError> {
    upsert_state_status(
        pool,
        owner_id,
        pitch_id,
        PitchStatus::Locked,
        Some(version_id),
    )
    .await
}

/// Reopen a locked pitch for refinement — flips status back to InProgress
/// so the next lock-in creates vN+1. Preserves `current_version_id`.
pub async fn reopen(pool: &PgPool, owner_id: &str, pitch_id: &str) -> Result<(), AppError> {
    upsert_state_status(pool, owner_id, pitch_id, PitchStatus::InProgress, None).await
}

/// Wipe per-user state — DELETE returns the pitch to the implicit
/// "not_started" view (COALESCE in the JOIN reads). Past versions stay in
/// `pitch_versions` (immutable history) but no longer point to a current.
pub async fn reset(pool: &PgPool, owner_id: &str, pitch_id: &str) -> Result<(), AppError> {
    sqlx::query("DELETE FROM pitch_user_state WHERE owner_id = $1 AND pitch_id = $2")
        .bind(owner_id)
        .bind(pitch_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Shared UPSERT body for `set_current_version` (Locked + version) and
/// `reopen` (InProgress, keep existing version). The `version_id`
/// argument, when `Some`, overrides the existing pointer; when `None`,
/// existing pointer is preserved on UPDATE.
async fn upsert_state_status(
    pool: &PgPool,
    owner_id: &str,
    pitch_id: &str,
    status: PitchStatus,
    version_id: Option<&str>,
) -> Result<(), AppError> {
    sqlx::query(
        r#"INSERT INTO pitch_user_state
               (owner_id, pitch_id, status, current_version_id, chat, updated_at)
           VALUES ($1, $2, $3, $4, '[]'::jsonb, NOW())
           ON CONFLICT (owner_id, pitch_id) DO UPDATE
             SET status = EXCLUDED.status,
                 current_version_id =
                     COALESCE(EXCLUDED.current_version_id, pitch_user_state.current_version_id),
                 updated_at = NOW()"#,
    )
    .bind(owner_id)
    .bind(pitch_id)
    .bind(status.as_str())
    .bind(version_id)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
#[path = "pitch_store_tests.rs"]
mod tests;

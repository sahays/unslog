//! Store for the Postgres `stories` table — Story Builder shells.
//!
//! Read/write surface:
//! * `get` / `find_or_404` — lookups.
//! * `push_turn` — append chat turn, bump updated_at (capacity-checked).
//! * `set_current_version` — repoint head + flip status to Complete.
//! * `set_status` / `set_mode` — small status / coach-mode flips.
//! * `delete_cascade` — owner-scoped DELETE; FK CASCADE (migration 0001)
//!   wipes the dependent story_versions.
//! * `list_for_landing` / `list_siblings` / `list_in_progress` — projected
//!   reads for the index tile grid, the show-page sidebar, and the home feed.
//!
//! Every read filters by `owner_id`; every write sets it. The handler
//! layer threads `CurrentUser::id` through.

use chrono::{DateTime, Utc};
use sqlx::types::Json;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{ChatTurn, Difficulty, Story, StoryStatus};

#[path = "story_store_row.rs"]
mod row;
use row::{parse_status, StoryListRowSql, StoryRow};
pub use row::{SiblingRow, StoryListRow};

/// Hard ceiling on the embedded chat array. Hitting the cap surfaces a
/// 400 telling the user to lock in and refine from there.
pub const MAX_CHAT_TURNS: usize = 100;

/// Columns selected by every full-row read. Centralized so a schema add
/// only edits one place.
const STORY_COLS: &str = r#"id, owner_id, competency_id, status, mode,
    current_version_id,
    chat AS "chat: Json<Vec<ChatTurn>>",
    created_at, updated_at"#;

/// Pure capacity guard for [`push_turn`]. Split out so we can unit-test
/// the cap without a live `&PgPool` handle.
fn check_chat_capacity(current_len: usize) -> Result<(), AppError> {
    if current_len >= MAX_CHAT_TURNS {
        return Err(AppError::BadRequest(format!(
            "chat is full ({MAX_CHAT_TURNS} turns) — lock this version in and refine from there"
        )));
    }
    Ok(())
}

// ── Reads ────────────────────────────────────────────────────────────────

pub async fn get(pool: &PgPool, owner_id: &str, id: &str) -> Result<Option<Story>, AppError> {
    let sql = format!("SELECT {STORY_COLS} FROM stories WHERE owner_id = $1 AND id = $2");
    let row: Option<StoryRow> = sqlx::query_as(&sql)
        .bind(owner_id)
        .bind(id)
        .fetch_optional(pool)
        .await?;
    row.map(StoryRow::try_into_story).transpose()
}

pub async fn find_or_404(pool: &PgPool, owner_id: &str, id: &str) -> Result<Story, AppError> {
    get(pool, owner_id, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("story {id}")))
}

/// Projected list for the landing page — every story, newest-first.
pub async fn list_for_landing(
    pool: &PgPool,
    owner_id: &str,
) -> Result<Vec<StoryListRow>, AppError> {
    fetch_landing(pool, owner_id, None).await
}

/// In-progress stories only, newest-first. Powers the home page's
/// "In progress" feed.
pub async fn list_in_progress(
    pool: &PgPool,
    owner_id: &str,
) -> Result<Vec<StoryListRow>, AppError> {
    fetch_landing(pool, owner_id, Some(StoryStatus::InProgress)).await
}

/// Shared body for `list_for_landing` + `list_in_progress`. `status_filter`
/// is `None` for "all stories" and `Some(status)` for "only this status".
async fn fetch_landing(
    pool: &PgPool,
    owner_id: &str,
    status_filter: Option<StoryStatus>,
) -> Result<Vec<StoryListRow>, AppError> {
    let sql = "SELECT id, competency_id, status, current_version_id, updated_at \
               FROM stories \
               WHERE owner_id = $1 AND ($2::text IS NULL OR status = $2) \
               ORDER BY updated_at DESC";
    let rows: Vec<StoryListRowSql> = sqlx::query_as(sql)
        .bind(owner_id)
        .bind(status_filter.map(|s| s.as_str()))
        .fetch_all(pool)
        .await?;
    rows.into_iter()
        .map(StoryListRowSql::try_into_landing_row)
        .collect()
}

/// Every story whose status is `complete`. Used by the eval gold-set
/// extractor; small N (≤ a few hundred) so a full scan is fine.
pub async fn list_completed(pool: &PgPool, owner_id: &str) -> Result<Vec<Story>, AppError> {
    let sql = format!(
        "SELECT {STORY_COLS} FROM stories \
         WHERE owner_id = $1 AND status = $2 \
         ORDER BY updated_at DESC"
    );
    let rows: Vec<StoryRow> = sqlx::query_as(&sql)
        .bind(owner_id)
        .bind(StoryStatus::Complete.as_str())
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(StoryRow::try_into_story).collect()
}

/// Other stories for the same competency, excluding `story_id`. Sorted by
/// most-recently-updated. Powers the side panel on the show page.
pub async fn list_siblings(
    pool: &PgPool,
    owner_id: &str,
    story_id: &str,
    competency_id: &str,
) -> Result<Vec<SiblingRow>, AppError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: String,
        status: String,
        updated_at: DateTime<Utc>,
    }
    let rows: Vec<Row> = sqlx::query_as(
        r#"SELECT id, status, updated_at
           FROM stories
           WHERE owner_id = $1 AND competency_id = $2 AND id <> $3
           ORDER BY updated_at DESC"#,
    )
    .bind(owner_id)
    .bind(competency_id)
    .bind(story_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            Ok(SiblingRow {
                id: r.id,
                status: parse_status(&r.status)?,
                updated_at: r.updated_at,
            })
        })
        .collect()
}

// ── Writes ───────────────────────────────────────────────────────────────

/// Insert a freshly-built story row. Caller mints `story.id` via
/// `Story::new_id` and sets `story.owner_id`.
pub async fn insert(pool: &PgPool, story: &Story) -> Result<(), AppError> {
    sqlx::query(
        r#"INSERT INTO stories
           (id, owner_id, competency_id, status, mode, current_version_id,
            chat, created_at, updated_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"#,
    )
    .bind(&story.id)
    .bind(&story.owner_id)
    .bind(&story.competency_id)
    .bind(story.status.as_str())
    .bind(story.mode.as_str())
    .bind(&story.current_version_id)
    .bind(Json(&story.chat))
    .bind(story.created_at)
    .bind(story.updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Append a chat turn and bump `updated_at`. Mirrors
/// [`crate::services::pitch_store::push_turn`]. Refuses to grow the
/// embedded `chat[]` past [`MAX_CHAT_TURNS`] turns.
pub async fn push_turn(pool: &PgPool, story: &mut Story, turn: ChatTurn) -> Result<(), AppError> {
    check_chat_capacity(story.chat.len())?;
    let now = chrono::Utc::now();
    let turn_json = serde_json::to_value(&turn).map_err(|e| AppError::Other(e.into()))?;
    sqlx::query(
        r#"UPDATE stories
           SET chat = chat || $3::jsonb, updated_at = $4
           WHERE owner_id = $1 AND id = $2"#,
    )
    .bind(&story.owner_id)
    .bind(&story.id)
    .bind(turn_json)
    .bind(now)
    .execute(pool)
    .await?;
    story.chat.push(turn);
    story.updated_at = now;
    Ok(())
}

/// Repoint `current_version_id` and flip status to `complete` in one
/// owner-scoped UPDATE. Called by `story_lockin::generate_and_save`.
pub async fn set_current_version(
    pool: &PgPool,
    owner_id: &str,
    story_id: &str,
    version_id: &str,
) -> Result<(), AppError> {
    sqlx::query(
        r#"UPDATE stories
           SET current_version_id = $3, status = $4, updated_at = NOW()
           WHERE owner_id = $1 AND id = $2"#,
    )
    .bind(owner_id)
    .bind(story_id)
    .bind(version_id)
    .bind(StoryStatus::Complete.as_str())
    .execute(pool)
    .await?;
    Ok(())
}

/// Flip status without other side-effects. Used by `continue_chat` to
/// reopen a completed story for refinement.
pub async fn set_status(
    pool: &PgPool,
    owner_id: &str,
    story_id: &str,
    status: StoryStatus,
) -> Result<(), AppError> {
    update_scalar(pool, owner_id, story_id, "status", status.as_str()).await
}

/// Update coach mode for this story. New mode takes effect on the next
/// chat turn (which loads `story.mode` to pick the matching prompt).
pub async fn set_mode(
    pool: &PgPool,
    owner_id: &str,
    story_id: &str,
    mode: Difficulty,
) -> Result<(), AppError> {
    update_scalar(pool, owner_id, story_id, "mode", mode.as_str()).await
}

/// Shared body for the small single-column flips (`status`, `mode`).
/// `column` is a literal — never user input — so direct interpolation is
/// safe; values still go through bind.
async fn update_scalar(
    pool: &PgPool,
    owner_id: &str,
    story_id: &str,
    column: &'static str,
    value: &str,
) -> Result<(), AppError> {
    let sql = format!(
        "UPDATE stories SET {column} = $3, updated_at = NOW() \
         WHERE owner_id = $1 AND id = $2"
    );
    sqlx::query(&sql)
        .bind(owner_id)
        .bind(story_id)
        .bind(value)
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete the story. The Postgres FK CASCADE (migration 0001) drops the
/// dependent `story_versions` rows in the same transaction. Owner-scoped.
pub async fn delete_cascade(pool: &PgPool, owner_id: &str, story_id: &str) -> Result<(), AppError> {
    sqlx::query("DELETE FROM stories WHERE owner_id = $1 AND id = $2")
        .bind(owner_id)
        .bind(story_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
#[path = "story_store_tests.rs"]
mod tests;

//! Journal entries — plain CRUD over the Postgres `journal_entries`
//! table. No taxonomy, no AI; the user owns the text end-to-end.
//!
//! Every read is scoped by `owner_id` so a future multi-user world sees
//! only its own rows; during this transition the owner is always
//! [`crate::services::current_owner::TEMP_OWNER_ID`].
//!
//! Persistence goes through [`JournalSource`] so the small bit of logic
//! that lives in free functions below (timestamp bumps, NotFound mapping,
//! archive marker) is unit-testable with `mockall::automock`.

use async_trait::async_trait;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::JournalEntry;
use crate::services::current_owner::TEMP_OWNER_ID;

/// Persistence seam — owner-scoped operations on the journal table.
/// Production impl is [`PgJournalSource`]; tests inject `MockJournalSource`.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait JournalSource: Send + Sync {
    /// Active (non-archived) entries for `owner_id`, most-recently-updated first.
    async fn find_active(&self, owner_id: &str) -> Result<Vec<JournalEntry>, AppError>;
    async fn find_one(&self, owner_id: &str, id: &str) -> Result<Option<JournalEntry>, AppError>;
    async fn insert(&self, entry: &JournalEntry) -> Result<(), AppError>;
    async fn replace(&self, entry: &JournalEntry) -> Result<(), AppError>;
}

/// Production `JournalSource` — owns a `&PgPool` for the duration of one
/// request. Cheap to construct (the pool is a clone of an `Arc`).
pub struct PgJournalSource<'a> {
    pub pool: &'a PgPool,
}

#[async_trait]
impl<'a> JournalSource for PgJournalSource<'a> {
    async fn find_active(&self, owner_id: &str) -> Result<Vec<JournalEntry>, AppError> {
        let rows = sqlx::query_as!(
            JournalEntry,
            r#"
            SELECT id, owner_id, title, body, created_at, updated_at, archived_at
            FROM journal_entries
            WHERE owner_id = $1 AND archived_at IS NULL
            ORDER BY updated_at DESC
            "#,
            owner_id,
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows)
    }

    async fn find_one(&self, owner_id: &str, id: &str) -> Result<Option<JournalEntry>, AppError> {
        let row = sqlx::query_as!(
            JournalEntry,
            r#"
            SELECT id, owner_id, title, body, created_at, updated_at, archived_at
            FROM journal_entries
            WHERE owner_id = $1 AND id = $2
            "#,
            owner_id,
            id,
        )
        .fetch_optional(self.pool)
        .await?;
        Ok(row)
    }

    async fn insert(&self, entry: &JournalEntry) -> Result<(), AppError> {
        sqlx::query!(
            r#"
            INSERT INTO journal_entries
                (id, owner_id, title, body, created_at, updated_at, archived_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
            entry.id,
            entry.owner_id,
            entry.title,
            entry.body,
            entry.created_at,
            entry.updated_at,
            entry.archived_at,
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }

    async fn replace(&self, entry: &JournalEntry) -> Result<(), AppError> {
        sqlx::query!(
            r#"
            UPDATE journal_entries
            SET title = $3,
                body = $4,
                updated_at = $5,
                archived_at = $6
            WHERE owner_id = $1 AND id = $2
            "#,
            entry.owner_id,
            entry.id,
            entry.title,
            entry.body,
            entry.updated_at,
            entry.archived_at,
        )
        .execute(self.pool)
        .await?;
        Ok(())
    }
}

// ── Pool-facing convenience wrappers (route layer calls these) ──────────

pub async fn list_active(pool: &PgPool) -> Result<Vec<JournalEntry>, AppError> {
    list_active_via(&PgJournalSource { pool }).await
}

pub async fn get(pool: &PgPool, id: &str) -> Result<Option<JournalEntry>, AppError> {
    get_via(&PgJournalSource { pool }, id).await
}

pub async fn create(pool: &PgPool, title: String, body: String) -> Result<JournalEntry, AppError> {
    create_via(&PgJournalSource { pool }, title, body).await
}

pub async fn update(
    pool: &PgPool,
    id: &str,
    title: String,
    body: String,
) -> Result<JournalEntry, AppError> {
    update_via(&PgJournalSource { pool }, id, title, body).await
}

pub async fn archive(pool: &PgPool, id: &str) -> Result<(), AppError> {
    archive_via(&PgJournalSource { pool }, id).await
}

// ── Trait-generic logic (the mockable surface) ──────────────────────────

pub async fn list_active_via(deps: &dyn JournalSource) -> Result<Vec<JournalEntry>, AppError> {
    deps.find_active(TEMP_OWNER_ID).await
}

pub async fn get_via(deps: &dyn JournalSource, id: &str) -> Result<Option<JournalEntry>, AppError> {
    deps.find_one(TEMP_OWNER_ID, id).await
}

pub async fn create_via(
    deps: &dyn JournalSource,
    title: String,
    body: String,
) -> Result<JournalEntry, AppError> {
    let entry = JournalEntry::new(TEMP_OWNER_ID.to_string(), title, body);
    deps.insert(&entry).await?;
    Ok(entry)
}

/// Load → mutate → save with a fresh `updated_at`. Missing id → `NotFound`.
pub async fn update_via(
    deps: &dyn JournalSource,
    id: &str,
    title: String,
    body: String,
) -> Result<JournalEntry, AppError> {
    let mut entry = load_existing(deps, id).await?;
    entry.title = title;
    entry.body = body;
    entry.updated_at = chrono::Utc::now();
    deps.replace(&entry).await?;
    Ok(entry)
}

/// Mark the entry archived (hidden from the list view). Idempotent: archiving
/// an already-archived entry refreshes the timestamp.
pub async fn archive_via(deps: &dyn JournalSource, id: &str) -> Result<(), AppError> {
    let mut entry = load_existing(deps, id).await?;
    entry.archived_at = Some(chrono::Utc::now());
    deps.replace(&entry).await?;
    Ok(())
}

async fn load_existing(deps: &dyn JournalSource, id: &str) -> Result<JournalEntry, AppError> {
    deps.find_one(TEMP_OWNER_ID, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("journal entry {id}")))
}

#[cfg(test)]
#[path = "journal_store_tests.rs"]
mod tests;

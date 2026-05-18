//! Journal entries — plain CRUD over the `journal_entries` collection.
//! No taxonomy, no AI; the user owns the text end-to-end.
//!
//! Mongo access goes through [`JournalSource`] so the free functions below
//! (which carry the small bit of logic — id generation, timestamp bumps,
//! NotFound mapping, archive marker) can be unit-tested with `mockall::automock`.

use async_trait::async_trait;
use futures::TryStreamExt;
use mongodb::options::FindOptions;
use mongodb::Database;

use crate::error::AppError;
use crate::models::JournalEntry;

/// Persistence seam — primitive Mongo operations on the journal collection.
/// Production impl is `mongodb::Database`; tests inject `MockJournalSource`.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait JournalSource: Send + Sync {
    /// Active (non-archived) entries, most-recently-updated first.
    async fn find_active(&self) -> Result<Vec<JournalEntry>, AppError>;
    async fn find_one(&self, id: &str) -> Result<Option<JournalEntry>, AppError>;
    async fn insert(&self, entry: &JournalEntry) -> Result<(), AppError>;
    async fn replace(&self, entry: &JournalEntry) -> Result<(), AppError>;
}

#[async_trait]
impl JournalSource for Database {
    async fn find_active(&self) -> Result<Vec<JournalEntry>, AppError> {
        let opts = FindOptions::builder()
            .sort(bson::doc! { "updated_at": -1 })
            .build();
        // Mongo equality `null` matches both explicit null AND missing field,
        // so this works for entries written before `archived_at` existed.
        let cursor = self
            .collection::<JournalEntry>(JournalEntry::COLLECTION)
            .find(bson::doc! { "archived_at": null })
            .with_options(opts)
            .await?;
        Ok(cursor.try_collect().await?)
    }

    async fn find_one(&self, id: &str) -> Result<Option<JournalEntry>, AppError> {
        Ok(self
            .collection::<JournalEntry>(JournalEntry::COLLECTION)
            .find_one(bson::doc! { "_id": id })
            .await?)
    }

    async fn insert(&self, entry: &JournalEntry) -> Result<(), AppError> {
        self.collection::<JournalEntry>(JournalEntry::COLLECTION)
            .insert_one(entry)
            .await?;
        Ok(())
    }

    async fn replace(&self, entry: &JournalEntry) -> Result<(), AppError> {
        self.collection::<JournalEntry>(JournalEntry::COLLECTION)
            .replace_one(bson::doc! { "_id": &entry.id }, entry)
            .await?;
        Ok(())
    }
}

pub async fn list_active(deps: &dyn JournalSource) -> Result<Vec<JournalEntry>, AppError> {
    deps.find_active().await
}

pub async fn get(deps: &dyn JournalSource, id: &str) -> Result<Option<JournalEntry>, AppError> {
    deps.find_one(id).await
}

pub async fn create(
    deps: &dyn JournalSource,
    title: String,
    body: String,
) -> Result<JournalEntry, AppError> {
    let entry = JournalEntry::new(title, body);
    deps.insert(&entry).await?;
    Ok(entry)
}

/// Load → mutate → save with a fresh `updated_at`. Missing id → `NotFound`.
pub async fn update(
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
pub async fn archive(deps: &dyn JournalSource, id: &str) -> Result<(), AppError> {
    let mut entry = load_existing(deps, id).await?;
    entry.archived_at = Some(chrono::Utc::now());
    deps.replace(&entry).await?;
    Ok(())
}

async fn load_existing(deps: &dyn JournalSource, id: &str) -> Result<JournalEntry, AppError> {
    deps.find_one(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("journal entry {id}")))
}

#[cfg(test)]
#[path = "journal_store_tests.rs"]
mod tests;

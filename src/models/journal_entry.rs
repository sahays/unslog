use serde::{Deserialize, Serialize};

/// A free-form journal entry — the user's raw, unassisted record of a past
/// experience. Deliberately minimal: title, body, timestamps. No taxonomy,
/// no AI on this side, no coupling to stories.
///
/// Backed by Postgres `journal_entries` after Phase A. `owner_id` scopes
/// the row to a user; during this transition it is always `"usrmaster"`
/// and Phase 1 will replace the temp constant with the request-bound user.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct JournalEntry {
    #[serde(rename = "_id")]
    pub id: String,
    /// Set on Postgres rows; legacy Mongo docs (read by the one-shot
    /// importer) default to empty and are filled in by the import code
    /// before insert.
    #[serde(default)]
    pub owner_id: String,
    pub title: String,
    pub body: String,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// `Some(ts)` = archived (hidden from the list view) since `ts`. `None` =
    /// active. Archive is reversible at the data layer; v1 has no UI to
    /// unarchive.
    #[serde(default, with = "crate::models::datetime_compat::optional")]
    pub archived_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Max chars to show as the card preview on the list page.
const EXCERPT_CHARS: usize = 240;

impl JournalEntry {
    /// Legacy Mongo collection name. Retained for symmetry / docs only.
    pub const COLLECTION: &'static str = "journal_entries";

    pub fn new(owner_id: String, title: String, body: String) -> Self {
        let now = chrono::Utc::now();
        Self {
            id: crate::services::id_gen::new(crate::services::id_gen::Kind::JournalEntry),
            owner_id,
            title,
            body,
            created_at: now,
            updated_at: now,
            archived_at: None,
        }
    }

    /// First [`EXCERPT_CHARS`] of the body for the card preview. Truncates by
    /// char count (USVs) and appends an ellipsis if cut.
    pub fn excerpt(&self) -> String {
        let trimmed = self.body.trim();
        if trimmed.chars().count() <= EXCERPT_CHARS {
            return trimmed.to_string();
        }
        let mut out: String = trimmed.chars().take(EXCERPT_CHARS).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
#[path = "journal_entry_tests.rs"]
mod tests;

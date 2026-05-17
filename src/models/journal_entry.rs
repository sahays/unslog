use serde::{Deserialize, Serialize};

/// A free-form journal entry — the user's raw, unassisted record of a past
/// experience. Deliberately minimal: title, body, timestamps. No taxonomy,
/// no AI on this side, no coupling to stories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    #[serde(rename = "_id")]
    pub id: String,
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
    pub const COLLECTION: &'static str = "journal_entries";

    pub fn new(title: String, body: String) -> Self {
        let now = chrono::Utc::now();
        Self {
            id: uuid::Uuid::now_v7().to_string(),
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

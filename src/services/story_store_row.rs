//! Row adapters that bridge Postgres `stories` rows to in-memory `Story`
//! and the lighter [`StoryListRow`] projection. Kept in its own file so the
//! main `story_store` stays under the 300-LOC budget.

use chrono::{DateTime, Utc};
use sqlx::types::Json;

use crate::error::AppError;
use crate::models::{ChatTurn, Difficulty, Story, StoryStatus};

pub fn parse_status(s: &str) -> Result<StoryStatus, AppError> {
    StoryStatus::parse(s)
        .ok_or_else(|| AppError::Other(anyhow::anyhow!("stories.status {s:?} not in CHECK set")))
}

pub fn parse_mode(s: &str) -> Result<Difficulty, AppError> {
    Difficulty::parse(s)
        .ok_or_else(|| AppError::Other(anyhow::anyhow!("stories.mode {s:?} not in CHECK set")))
}

#[derive(sqlx::FromRow)]
pub struct StoryRow {
    pub id: String,
    pub owner_id: String,
    pub competency_id: String,
    pub status: String,
    pub mode: String,
    pub current_version_id: Option<String>,
    pub chat: Json<Vec<ChatTurn>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl StoryRow {
    pub fn try_into_story(self) -> Result<Story, AppError> {
        Ok(Story {
            id: self.id,
            owner_id: self.owner_id,
            competency_id: self.competency_id,
            status: parse_status(&self.status)?,
            mode: parse_mode(&self.mode)?,
            current_version_id: self.current_version_id,
            chat: self.chat.0,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

/// Projected row used by the landing page tile grid + completed-card list
/// and the home feed. The embedded chat is excluded.
pub struct StoryListRow {
    pub id: String,
    pub competency_id: String,
    pub status: StoryStatus,
    pub current_version_id: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
pub struct StoryListRowSql {
    pub id: String,
    pub competency_id: String,
    pub status: String,
    pub current_version_id: Option<String>,
    pub updated_at: DateTime<Utc>,
}

impl StoryListRowSql {
    pub fn try_into_landing_row(self) -> Result<StoryListRow, AppError> {
        Ok(StoryListRow {
            id: self.id,
            competency_id: self.competency_id,
            status: parse_status(&self.status)?,
            current_version_id: self.current_version_id,
            updated_at: self.updated_at,
        })
    }
}

/// Sibling row used by the show-page side panel — parent-scoped to one
/// competency, excluding the current id.
pub struct SiblingRow {
    pub id: String,
    pub status: StoryStatus,
    pub updated_at: DateTime<Utc>,
}

use serde::{Deserialize, Serialize};

use crate::services::id_gen::{self, Kind};

pub const PROMPT_NAMES: &[&str] = &[
    "critique",
    "research",
    "summary",
    "story_chat",
    "story_chat_collaborative",
    "story_summarize",
    "story_refine_open",
    "story_spoken",
    "pitch_chat",
    "pitch_lockin",
];

pub fn is_valid_prompt_name(name: &str) -> bool {
    PROMPT_NAMES.contains(&name)
}

/// Catalog row pointing at the active version of a prompt. Backed by
/// Postgres `prompts` table — primary key is the prompt `name`, which is
/// also the public identifier surfaced in `/agents/<name>`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Prompt {
    pub name: String,
    pub current_version_id: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Immutable prompt-body snapshot. New rows on every Save / Restore; the
/// active version is selected via `prompts.current_version_id`. Backed by
/// Postgres `prompt_versions` table.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PromptVersion {
    pub id: String,
    pub prompt_name: String,
    pub body: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub restored_from: Option<String>,
}

impl PromptVersion {
    /// Build a fresh `PromptVersion` with a freshly-minted prefixed id.
    /// Migration 0001 pins the id format to `prv` + 6 lowercase alphanums.
    pub fn new(prompt_name: String, body: String, restored_from: Option<String>) -> Self {
        Self {
            id: id_gen::new(Kind::PromptVersion),
            prompt_name,
            body,
            created_at: chrono::Utc::now(),
            restored_from,
        }
    }
}

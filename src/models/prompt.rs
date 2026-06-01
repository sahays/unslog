use serde::{Deserialize, Serialize};

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
/// Postgres `prompts` table after Phase A — primary key is the prompt
/// `name`, which is also the public identifier surfaced in `/agents/<name>`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Prompt {
    #[serde(rename = "_id")]
    pub name: String,
    pub current_version_id: String,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl Prompt {
    pub const COLLECTION: &'static str = "prompts";
}

/// Immutable prompt-body snapshot. New rows on every Save / Restore; the
/// active version is selected via `prompts.current_version_id`. Backed by
/// Postgres `prompt_versions` table.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PromptVersion {
    #[serde(rename = "_id")]
    pub id: String,
    pub prompt_name: String,
    pub body: String,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub restored_from: Option<String>,
}

impl PromptVersion {
    pub const COLLECTION: &'static str = "prompt_versions";

    /// Build a fresh `PromptVersion` with a freshly-minted prefixed id.
    /// Migration 0001 pins the id format to `prv` + 6 lowercase alphanums.
    pub fn new(prompt_name: String, body: String, restored_from: Option<String>) -> Self {
        Self {
            id: crate::services::mongo_import::id_map::mint(
                crate::services::mongo_import::id_map::Kind::PromptVersion,
            ),
            prompt_name,
            body,
            created_at: chrono::Utc::now(),
            restored_from,
        }
    }
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

    pub fn new(prompt_name: String, body: String, restored_from: Option<String>) -> Self {
        Self {
            id: uuid::Uuid::now_v7().to_string(),
            prompt_name,
            body,
            created_at: chrono::Utc::now(),
            restored_from,
        }
    }
}

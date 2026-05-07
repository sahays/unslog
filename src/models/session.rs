use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Ended,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSnapshot {
    pub stt: String,
    pub tts: String,
    pub critique: String,
    pub research: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptSnapshot {
    pub critique: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    #[serde(rename = "_id")]
    pub id: String,
    pub company_id: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub ended_at: Option<chrono::DateTime<chrono::Utc>>,
    pub status: SessionStatus,
    pub model_snapshot: ModelSnapshot,
    pub prompt_snapshot: PromptSnapshot,
    #[serde(default)]
    pub voice_critique_enabled: bool,
    #[serde(default)]
    pub current_question_id: Option<String>,
    #[serde(default)]
    pub current_question_text: Option<String>,
    #[serde(default)]
    pub current_question_audio_path: Option<String>,
}

impl Session {
    pub const COLLECTION: &'static str = "sessions";
}

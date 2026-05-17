use serde::{Deserialize, Serialize};

use crate::models::Role;

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
    #[serde(default)]
    pub tts_voice: String,
    /// Snapshot of the accent steering at session start (e.g. `en-GB`).
    /// Empty = use the TTS model's default voice characteristics. See
    /// `Settings::tts_language` for the source of truth.
    #[serde(default)]
    pub tts_language: String,
    #[serde(default)]
    pub tts_speed: Option<f32>,
    /// Snapshot of the cheap classifier/curator model. Defaults to
    /// "google/gemini-2.5-flash" for sessions started before this field
    /// existed, so legacy sessions still have a sensible value.
    #[serde(default = "default_lite_snapshot")]
    pub lite: String,
}

fn default_lite_snapshot() -> String {
    "google/gemini-2.5-flash".to_string()
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
    /// Anchor company. For single-company sessions, this is the only entry
    /// in `selected_company_ids`. For cross-company sessions, this points at
    /// the primary one (used for any "primary packet" fallback in critique).
    pub company_id: String,
    /// Canonical role this session is practising. Drives the question pool.
    /// Defaulted for sessions started before this field existed.
    #[serde(default = "default_role")]
    pub role: Role,
    /// Companies this session draws questions from. For single-company entry
    /// = `[company_id]`; for cross-company `/practice` = multiple. Empty
    /// (legacy) is interpreted as `[company_id]`.
    #[serde(default)]
    pub selected_company_ids: Vec<String>,
    /// Curator's pre-picked question IDs in answer order. Empty for legacy
    /// sessions; the curator falls back to per-call random for those.
    #[serde(default)]
    pub curated_question_ids: Vec<String>,
    /// One-line "today's focus" message from the curator.
    #[serde(default)]
    pub focus_line: String,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub started_at: chrono::DateTime<chrono::Utc>,
    #[serde(default, with = "crate::models::datetime_compat::optional")]
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

fn default_role() -> Role {
    Role::SolutionsArchitect
}

impl Session {
    pub const COLLECTION: &'static str = "sessions";
}

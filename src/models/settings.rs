use serde::{Deserialize, Serialize};

/// Singleton settings document. `_id` is always `"default"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(rename = "_id")]
    pub id: String,
    pub critique_model: String,
    pub research_model: String,
    pub stt_model: String,
    pub tts_model: String,
    pub tts_voice: String,
    /// 0.25–4.0; `None` means "let the TTS endpoint use its own default".
    #[serde(default)]
    pub tts_speed: Option<f32>,
    /// Cheap fast model used for question categorization + session curation.
    /// Defaults to `google/gemini-2.5-flash` on first read.
    #[serde(default = "default_lite_model")]
    pub lite_model: String,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

fn default_lite_model() -> String {
    "google/gemini-2.5-flash".to_string()
}

impl Settings {
    pub const COLLECTION: &'static str = "settings";
    pub const SINGLETON_ID: &'static str = "default";
}

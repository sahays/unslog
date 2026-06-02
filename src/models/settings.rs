use serde::{Deserialize, Serialize};

/// Singleton settings document. Backed by Postgres `settings` table after
/// Phase A; `id` is pinned to `Settings::SINGLETON_ID` (`"singleton"`) so a
/// stray INSERT can never create a second row.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Settings {
    #[serde(rename = "_id")]
    pub id: String,
    pub critique_model: String,
    pub research_model: String,
    pub stt_model: String,
    pub tts_model: String,
    pub tts_voice: String,
    /// Locale steering for TTS (e.g. `en-US`, `en-GB`, `en-IN`). Sent to
    /// gpt-4o-mini-tts via the `instructions` field to bias accent. Empty
    /// means "use the model's default accent". Defaulted so legacy rows still
    /// deserialize.
    #[serde(default)]
    pub tts_language: String,
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
    /// Legacy Mongo collection name. Retained for symmetry / docs only.
    pub const COLLECTION: &'static str = "settings";
    /// The one and only row's id. Pinned by a CHECK on the Postgres table.
    pub const SINGLETON_ID: &'static str = "singleton";
}

use askama::Template;
use axum::extract::{Form, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;

use crate::error::AppError;
use crate::filters; // Custom Askama filters used by templates below.
use crate::models::Settings;
use crate::services::{openrouter_models::ModelInfo, settings_store, text_validation};
use crate::startup::AppState;

/// Sanity cap on OpenRouter model identifiers (e.g. `anthropic/claude-sonnet-4-5`).
/// Real ids are <50 chars; cap generously without letting an attacker stuff
/// a 1 MB string into the prompt-routing field.
const MAX_MODEL_ID: usize = 200;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/settings", get(show).post(save))
        .route("/settings/refresh-models", post(refresh_models))
}

#[derive(Template)]
#[template(path = "settings/index.html")]
struct SettingsTemplate {
    settings: Settings,
    chat_models: Vec<ModelInfo>,
    audio_in_models: Vec<ModelInfo>,
    audio_out_models: Vec<ModelInfo>,
    /// Wrap String in a struct so Askama's field-access pattern (mirroring
    /// `m.id == settings.critique_model` elsewhere) produces a working
    /// comparison; bare `String` in a loop comes through as `&String` which
    /// doesn't `PartialEq` against `String`.
    tts_voices: Vec<VoiceOption>,
    tts_languages: &'static [LanguageOption],
    models_error: Option<String>,
    openrouter_configured: bool,
}

struct VoiceOption {
    pub id: String,
}

struct LanguageOption {
    pub code: &'static str,
    pub label: &'static str,
}

const LANGUAGE_OPTIONS: &[LanguageOption] = &[
    LanguageOption {
        code: "en-US",
        label: "American (en-US)",
    },
    LanguageOption {
        code: "en-GB",
        label: "British (en-GB)",
    },
    LanguageOption {
        code: "en-IN",
        label: "Indian (en-IN)",
    },
    LanguageOption {
        code: "en-AU",
        label: "Australian (en-AU)",
    },
];

/// Restrict the cached /models list to a curated set of providers. The user
/// can still type any model ID via NSelect's `data-allow-custom` mode, so this
/// is a soft filter — a brand-new model from one of these providers shows up
/// after the next /models refresh, and anything else is reachable by typing.
const PREFERRED_PREFIXES: &[&str] = &[
    "google/gemini", // Gemini only — skip gemma, learnlm, palm
    "openai/",       // gpt-*, o1-*, o3-*, chatgpt-*
    "anthropic/",    // claude-*
    "deepseek/",     // deepseek-*
    "x-ai/",         // grok-*
    "qwen/",         // qwen-*, qwen3-*
];

fn is_preferred(id: &str) -> bool {
    PREFERRED_PREFIXES.iter().any(|p| id.starts_with(p))
}

async fn show(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let settings = state.settings_cache.get(&state.db).await?;

    let (mut chat_models, mut audio_in_models, mut audio_out_models, models_error) =
        if state.openrouter.configured() {
            match state.models_cache.get(&*state.openrouter).await {
                Ok(all) => {
                    let chat: Vec<_> = all
                        .iter()
                        .filter(|m| m.supports_text_chat() && is_preferred(&m.id))
                        .cloned()
                        .collect();
                    let ain: Vec<_> = all
                        .iter()
                        .filter(|m| m.supports_audio_in() && is_preferred(&m.id))
                        .cloned()
                        .collect();
                    let aout: Vec<_> = all
                        .iter()
                        .filter(|m| m.supports_audio_out() && is_preferred(&m.id))
                        .cloned()
                        .collect();
                    (chat, ain, aout, None)
                }
                Err(e) => (vec![], vec![], vec![], Some(e.to_string())),
            }
        } else {
            (
                vec![],
                vec![],
                vec![],
                Some("OPENROUTER_API_KEY not set — paste model IDs manually below.".into()),
            )
        };

    // If the user's currently-saved model isn't in the cached list (e.g. a
    // brand-new model just added at OpenRouter, or a custom value typed in
    // before the cache refreshed), inject a synthetic entry so the <select>
    // can render it as the selected option.
    ensure_present(&mut chat_models, &settings.critique_model);
    ensure_present(&mut chat_models, &settings.research_model);
    ensure_present(&mut chat_models, &settings.lite_model);
    ensure_present(&mut audio_in_models, &settings.stt_model);
    ensure_present(&mut audio_out_models, &settings.tts_model);

    crate::error::render_html(SettingsTemplate {
        settings,
        chat_models,
        audio_in_models,
        audio_out_models,
        tts_voices: OPENAI_TTS_VOICES
            .iter()
            .map(|s| VoiceOption {
                id: (*s).to_string(),
            })
            .collect(),
        tts_languages: LANGUAGE_OPTIONS,
        models_error,
        openrouter_configured: state.openrouter.configured(),
    })
}

fn ensure_present(list: &mut Vec<ModelInfo>, id: &str) {
    if id.is_empty() {
        return;
    }
    if list.iter().any(|m| m.id == id) {
        return;
    }
    list.insert(
        0,
        ModelInfo {
            id: id.to_string(),
            name: id.to_string(),
            architecture: Default::default(),
        },
    );
}

#[derive(Deserialize)]
pub struct SettingsForm {
    pub critique_model: String,
    pub research_model: String,
    pub stt_model: String,
    pub tts_model: String,
    pub tts_voice: String,
    #[serde(default)]
    pub tts_language: String,
    #[serde(default)]
    pub tts_speed: String,
    #[serde(default)]
    pub lite_model: String,
}

/// Allowed accent codes for the language selector. Anything else is rejected
/// at save time; the TTS layer treats empty as "use model default".
const ALLOWED_LANGUAGES: &[&str] = &["", "en-US", "en-GB", "en-IN", "en-AU"];

/// Voices we surface in the dropdown. `data-allow-custom` lets users type any
/// model-specific voice not on this list. These are OpenAI gpt-4o-mini-tts
/// voices as of 2025 — neutral steering, accent controlled separately via the
/// language setting / `instructions` field.
pub(crate) const OPENAI_TTS_VOICES: &[&str] = &[
    "alloy", "ash", "ballad", "coral", "echo", "fable", "onyx", "nova", "sage", "shimmer", "verse",
];

async fn save(
    State(state): State<AppState>,
    Form(form): Form<SettingsForm>,
) -> Result<Response, AppError> {
    let critique_model =
        text_validation::sanitize_short(&form.critique_model, MAX_MODEL_ID, "critique_model")?;
    let research_model =
        text_validation::sanitize_short(&form.research_model, MAX_MODEL_ID, "research_model")?;
    let stt_model = text_validation::sanitize_short(&form.stt_model, MAX_MODEL_ID, "stt_model")?;
    let tts_model = text_validation::sanitize_short(&form.tts_model, MAX_MODEL_ID, "tts_model")?;
    let tts_voice = text_validation::sanitize_short(&form.tts_voice, MAX_MODEL_ID, "tts_voice")?;
    let tts_language = form.tts_language.trim().to_string();
    if !ALLOWED_LANGUAGES.contains(&tts_language.as_str()) {
        return Err(AppError::BadRequest(format!(
            "unsupported tts_language `{tts_language}`"
        )));
    }
    let lite_raw = form.lite_model.trim();
    let lite_model = if lite_raw.is_empty() {
        crate::services::openrouter::DEFAULT_LITE_MODEL.to_string()
    } else {
        text_validation::sanitize_short(lite_raw, MAX_MODEL_ID, "lite_model")?
    };

    // `sanitize_short` above enforces non-empty per field — the old
    // "all model and voice fields are required" fallback is no longer needed.

    let speed_raw = form.tts_speed.trim();
    let tts_speed = if speed_raw.is_empty() {
        None
    } else {
        let v: f32 = speed_raw
            .parse()
            .map_err(|_| AppError::BadRequest("tts_speed must be a number".into()))?;
        if !(0.25..=4.0).contains(&v) {
            return Err(AppError::BadRequest(
                "tts_speed must be between 0.25 and 4.0".into(),
            ));
        }
        Some(v)
    };

    let next = Settings {
        id: Settings::SINGLETON_ID.to_string(),
        critique_model,
        research_model,
        stt_model,
        tts_model,
        tts_voice,
        tts_language,
        tts_speed,
        lite_model,
        updated_at: chrono::Utc::now(),
    };
    settings_store::save(&state.db, &next).await?;
    state.settings_cache.invalidate().await;
    tracing::info!(
        event = "settings.save",
        critique_model = %next.critique_model,
        research_model = %next.research_model,
        stt_model = %next.stt_model,
        tts_model = %next.tts_model,
        tts_voice = %next.tts_voice,
        tts_language = %next.tts_language,
        tts_speed = ?next.tts_speed,
        lite_model = %next.lite_model,
        "settings updated",
    );

    Ok(Redirect::to("/settings").into_response())
}

async fn refresh_models(State(state): State<AppState>) -> Result<Response, AppError> {
    state.models_cache.invalidate().await;
    Ok(Redirect::to("/settings").into_response())
}

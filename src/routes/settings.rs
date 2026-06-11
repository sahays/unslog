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
    // Merge the invites sub-router BEFORE applying the master-only
    // middleware layer so `/settings/invites/*` inherits the same gate
    // as `/settings`. Nesting under a fresh `Router` would skip the
    // layer because axum applies layers to the router they're attached
    // to, not to merged child routes that join later.
    Router::new()
        .route("/settings", get(show).post(save))
        .route("/settings/refresh-models", post(refresh_models))
        .merge(crate::routes::invites::routes())
        .layer(axum::middleware::from_fn(
            crate::middleware::require_master_middleware,
        ))
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
    /// Read-only view of the configured per-tier daily caps. Source of
    /// truth is `AppConfig` (env), surfaced here so the admin can see the
    /// active values without grepping `.env`.
    pro_request_cap_daily: u32,
    pro_max_request_cap_daily: u32,
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
    let settings = state.settings_cache.get(&state.pool).await?;

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
                    // TTS dropdown trusts OpenRouter's speech-modality
                    // classification (see supports_audio_out — strict to
                    // "speech") rather than an in-source curated list.
                    // Whatever OpenRouter exposes is what we offer.
                    let aout: Vec<_> = all
                        .iter()
                        .filter(|m| m.supports_audio_out())
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

    // Same pattern as ensure_present for model dropdowns: if the saved
    // voice isn't in the seed list (e.g. an Azure voice like
    // "en-US-Harper:MAI-Voice-2" for mai-voice-2), inject it so the
    // <select> can mark it selected on render. Without this the form
    // silently falls back to the empty placeholder after save and the
    // user thinks their voice didn't persist.
    let mut tts_voices: Vec<VoiceOption> = SUGGESTED_TTS_VOICES
        .iter()
        .map(|s| VoiceOption {
            id: (*s).to_string(),
        })
        .collect();
    if !settings.tts_voice.is_empty()
        && !tts_voices.iter().any(|v| v.id == settings.tts_voice)
    {
        tts_voices.insert(
            0,
            VoiceOption {
                id: settings.tts_voice.clone(),
            },
        );
    }

    crate::error::render_html(SettingsTemplate {
        settings,
        chat_models,
        audio_in_models,
        audio_out_models,
        tts_voices,
        tts_languages: LANGUAGE_OPTIONS,
        models_error,
        openrouter_configured: state.openrouter.configured(),
        pro_request_cap_daily: state.config.pro_request_cap_daily,
        pro_max_request_cap_daily: state.config.pro_max_request_cap_daily,
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

/// Seed suggestions for the voice <select>. The field is `data-allow-custom`
/// so the user can type any model-specific voice — this list is convenience
/// only, not a filter. Currently scoped to Azure Neural British + Indian
/// voices since `microsoft/mai-voice-2` is the primary TTS model and these
/// are the accent options the user actually uses; other working voices
/// (other locales, `kokoro` `bf_*`/`bm_*`/`if_*`/`im_*`, Grok `ara`/`eve`,
/// `sesame` `alloy`/`narrator`/…) are reachable by typing.
pub(crate) const SUGGESTED_TTS_VOICES: &[&str] = &[
    "en-GB-SoniaNeural",
    "en-GB-LibbyNeural",
    "en-GB-AbbiNeural",
    "en-GB-BellaNeural",
    "en-GB-MaisieNeural",
    "en-GB-OliviaNeural",
    "en-GB-RyanNeural",
    "en-GB-ThomasNeural",
    "en-GB-OliverNeural",
    "en-IN-NeerjaNeural",
    "en-IN-AnanyaNeural",
    "en-IN-KavyaNeural",
    "en-IN-PrabhatNeural",
    "en-IN-AaravNeural",
    "en-IN-KunalNeural",
    "en-IN-RehaanNeural",
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

    let tts_speed = parse_tts_speed(&form.tts_speed)?;

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
    settings_store::save(&state.pool, &next).await?;
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

/// Allowed TTS speed range from the OpenAI gpt-4o-mini-tts spec. Values
/// outside this band would be rejected by the upstream endpoint anyway,
/// but we reject at the trust boundary so the user sees a clean error
/// instead of a 4xx from OpenRouter that costs a billed retry.
const TTS_SPEED_MIN: f32 = 0.25;
const TTS_SPEED_MAX: f32 = 4.0;

/// Parse the `tts_speed` form field. Empty → `None` (use endpoint default).
/// Non-numeric → BadRequest. Out-of-range → BadRequest.
fn parse_tts_speed(raw: &str) -> Result<Option<f32>, AppError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let v: f32 = trimmed
        .parse()
        .map_err(|_| AppError::BadRequest("tts_speed must be a number".into()))?;
    if !(TTS_SPEED_MIN..=TTS_SPEED_MAX).contains(&v) {
        return Err(AppError::BadRequest(format!(
            "tts_speed must be between {TTS_SPEED_MIN} and {TTS_SPEED_MAX}"
        )));
    }
    Ok(Some(v))
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;

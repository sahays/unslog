use askama::Template;
use axum::extract::{Form, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;

use crate::error::AppError;
use crate::models::Settings;
use crate::services::{openrouter_models::ModelInfo, settings_store};
use crate::startup::AppState;

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
    models_error: Option<String>,
    openrouter_configured: bool,
}

async fn show(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let settings = settings_store::load(&state.db).await?;

    let (mut chat_models, mut audio_in_models, mut audio_out_models, models_error) =
        if state.openrouter.configured() {
            match state.models_cache.get(&state.openrouter).await {
                Ok(all) => {
                    let chat: Vec<_> = all
                        .iter()
                        .filter(|m| m.supports_text_chat())
                        .cloned()
                        .collect();
                    let ain: Vec<_> = all
                        .iter()
                        .filter(|m| m.supports_audio_in())
                        .cloned()
                        .collect();
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
    ensure_present(&mut audio_in_models, &settings.stt_model);
    ensure_present(&mut audio_out_models, &settings.tts_model);

    let body = SettingsTemplate {
        settings,
        chat_models,
        audio_in_models,
        audio_out_models,
        models_error,
        openrouter_configured: state.openrouter.configured(),
    }
    .render()
    .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(body))
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
    pub tts_speed: String,
}

async fn save(
    State(state): State<AppState>,
    Form(form): Form<SettingsForm>,
) -> Result<Response, AppError> {
    let trim = |s: String| s.trim().to_string();
    let critique_model = trim(form.critique_model);
    let research_model = trim(form.research_model);
    let stt_model = trim(form.stt_model);
    let tts_model = trim(form.tts_model);
    let tts_voice = trim(form.tts_voice);

    if critique_model.is_empty()
        || research_model.is_empty()
        || stt_model.is_empty()
        || tts_model.is_empty()
        || tts_voice.is_empty()
    {
        return Err(AppError::BadRequest(
            "all model and voice fields are required".into(),
        ));
    }

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
        tts_speed,
        updated_at: chrono::Utc::now(),
    };
    settings_store::save(&state.db, &next).await?;

    Ok(Redirect::to("/settings").into_response())
}

async fn refresh_models(State(state): State<AppState>) -> Result<Response, AppError> {
    state.models_cache.invalidate().await;
    Ok(Redirect::to("/settings").into_response())
}

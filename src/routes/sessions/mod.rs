//! `/sessions/*` and `/companies/:id/sessions` routes.
//!
//! Split into:
//! * `lifecycle` — start / show / review / next-question / end / delete /
//!   `advance_to_next` (the engine that picks the next curated question and
//!   TTSes it).
//! * `answer` — submit_answer / transcribe / regenerate-audio.
//!
//! Helpers shared between the two sit here. Children access them via
//! `super::name`; nothing here is `pub` outside the crate except
//! `advance_to_next`, which is re-exported for `routes::practice::start`.

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;

use crate::error::AppError;
use crate::models::{Company, Session, SessionStatus};
use crate::services::{company_store, sessions, summary, tts};
use crate::startup::AppState;

mod answer;
mod lifecycle;

// External callers expect `crate::routes::sessions::advance_to_next`.
pub(crate) use lifecycle::advance_to_next;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/companies/:id/sessions", post(lifecycle::start))
        .route("/sessions/:id", get(lifecycle::show))
        .route("/sessions/:id/review", get(lifecycle::review))
        .route(
            "/sessions/:id/next-question",
            post(lifecycle::next_question),
        )
        .route("/sessions/:id/transcribe", post(answer::transcribe))
        .route("/sessions/:id/answers", post(answer::submit_answer))
        .route("/sessions/:id/toggle-voice", post(toggle_voice))
        .route("/sessions/:id/end", post(end))
        .route("/sessions/:id/delete", post(delete_session))
        .route(
            "/sessions/:id/attempts/:eval_id/:n/regenerate-audio",
            post(answer::regenerate_critique_audio),
        )
}

// ── Helpers shared by lifecycle + answer ─────────────────────────────────

async fn load_session(state: &AppState, id: &str) -> Result<Session, AppError> {
    sessions::find_or_404(&state.db, id).await
}

async fn load_company(state: &AppState, id: &str) -> Result<Company, AppError> {
    company_store::find_or_404(&state.db, id).await
}

/// Synthesize `text` to MP3 in this session's recording dir at `filename`,
/// honoring the snapshotted model + voice + speed. Returns the absolute path
/// as a string for storage in Mongo.
async fn tts_to(
    state: &AppState,
    session: &Session,
    filename: &str,
    text: &str,
) -> Result<String, AppError> {
    if !state.openrouter.configured() {
        return Err(AppError::OpenRouterNotConfigured);
    }
    let dir =
        crate::recordings::session_dir(&state.config.data_dir, &session.company_id, &session.id);
    crate::recordings::ensure_dir(&dir).await?;
    let path = dir.join(filename);
    let voice = if session.model_snapshot.tts_voice.is_empty() {
        crate::services::openrouter::DEFAULT_TTS_VOICE
    } else {
        &session.model_snapshot.tts_voice
    };
    let instructions = tts::build_accent_instructions(&session.model_snapshot.tts_language);
    let path = tts::synthesize(
        &*state.openrouter,
        &session.model_snapshot.tts,
        voice,
        text,
        session.model_snapshot.tts_speed,
        &instructions,
        path,
    )
    .await?;
    Ok(path.to_string_lossy().into_owned())
}

fn critique_audio_filename(question_id: &str, attempt_n: u32) -> String {
    format!("critique_{question_id}_v{attempt_n}.mp3")
}

// ── Small lifecycle handlers ─────────────────────────────────────────────

async fn toggle_voice(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let session = load_session(&state, &id).await?;
    sessions::toggle_voice(&state.db, &session).await?;
    Ok(Redirect::to(&format!("/sessions/{id}")).into_response())
}

async fn end(State(state): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let session = load_session(&state, &id).await?;
    if session.status == SessionStatus::Active {
        let company = load_company(&state, &session.company_id).await?;
        tracing::info!(
            event = "session.end",
            session_id = %id,
            company_id = %session.company_id,
            "ending session, generating summary",
        );
        // Best-effort: if the LLM call fails (no key, model down), still let
        // the session end so the user isn't stuck with an unkillable session.
        let summary_ctx = summary::SummaryCtx {
            db: &state.db,
            pool: &state.pool,
        };
        if let Err(e) =
            summary::generate_and_save(&summary_ctx, &*state.openrouter, &session, &company).await
        {
            tracing::warn!(error = %e, session_id = %id, "summary generation failed; ending session anyway");
        }
    }
    sessions::end(&state.db, &id).await?;
    Ok(Redirect::to(&format!("/sessions/{id}")).into_response())
}

async fn delete_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let (session_deleted, evals_deleted, summaries_deleted) =
        sessions::delete_cascade(&state.db, &id).await?;
    tracing::info!(
        event = "session.delete",
        session_id = %id,
        session_deleted,
        evals_deleted,
        summaries_deleted,
        "session deleted",
    );
    Ok(Redirect::to("/practice").into_response())
}

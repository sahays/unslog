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
use crate::models::{Company, Evaluation, Session, SessionStatus, Summary};
use crate::services::{summary, tts};
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
    state
        .db
        .collection::<Session>(Session::COLLECTION)
        .find_one(bson::doc! { "_id": id })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("session {id}")))
}

async fn load_company(state: &AppState, id: &str) -> Result<Company, AppError> {
    crate::db::companies(&state.db)
        .find_one(bson::doc! { "_id": id })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("company {id}")))
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
    let path = tts::synthesize(
        &*state.openrouter,
        &session.model_snapshot.tts,
        voice,
        text,
        session.model_snapshot.tts_speed,
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
    state
        .db
        .collection::<Session>(Session::COLLECTION)
        .update_one(
            bson::doc! { "_id": &id },
            bson::doc! { "$set": { "voice_critique_enabled": !session.voice_critique_enabled } },
        )
        .await?;
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
        let summary_ctx = summary::SummaryCtx { db: &state.db };
        if let Err(e) =
            summary::generate_and_save(&summary_ctx, &*state.openrouter, &session, &company).await
        {
            tracing::warn!(error = %e, session_id = %id, "summary generation failed; ending session anyway");
        }
    }

    state
        .db
        .collection::<Session>(Session::COLLECTION)
        .update_one(
            bson::doc! { "_id": &id },
            bson::doc! { "$set": {
                "status": "ended",
                "ended_at": chrono::Utc::now().to_rfc3339(),
                "current_question_id": null,
                "current_question_text": null,
            } },
        )
        .await?;
    Ok(Redirect::to(&format!("/sessions/{id}")).into_response())
}

async fn delete_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let sessions = state.db.collection::<Session>(Session::COLLECTION);
    let evals = state.db.collection::<Evaluation>(Evaluation::COLLECTION);
    let summaries = state.db.collection::<Summary>(Summary::COLLECTION);

    let evals_deleted = evals
        .delete_many(bson::doc! { "session_id": &id })
        .await?
        .deleted_count;
    let summaries_deleted = summaries
        .delete_many(bson::doc! { "session_id": &id })
        .await?
        .deleted_count;
    let session_deleted = sessions
        .delete_one(bson::doc! { "_id": &id })
        .await?
        .deleted_count;

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

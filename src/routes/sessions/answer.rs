//! Answer flow: submit_answer (record an attempt + critique it), transcribe
//! (STT a recording), regenerate_critique_audio (re-TTS a stored critique).

use axum::extract::{Form, Path, State};
use axum::response::{IntoResponse, Json, Redirect, Response};
use axum_extra::extract::Multipart;
use serde::Deserialize;
use serde_json::json;

use crate::error::AppError;
use crate::models::{Attempt, Evaluation, SessionStatus};
use crate::services::{critique, evaluations, stt, summary};
use crate::startup::AppState;

#[derive(Deserialize)]
pub(super) struct AnswerForm {
    pub transcript: String,
    #[serde(default)]
    pub audio_path: Option<String>,
}

pub(super) async fn submit_answer(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Form(form): Form<AnswerForm>,
) -> Result<Response, AppError> {
    let answer = form.transcript.trim().to_string();
    if answer.is_empty() {
        return Err(AppError::BadRequest("answer is empty".into()));
    }
    let audio_path = form.audio_path.filter(|s| !s.is_empty());

    let session = super::load_session(&state, &id).await?;
    if session.status != SessionStatus::Active {
        return Err(AppError::BadRequest("session has ended".into()));
    }
    let qid = session
        .current_question_id
        .clone()
        .ok_or_else(|| AppError::BadRequest("pick a question first".into()))?;
    let qtext = session
        .current_question_text
        .clone()
        .ok_or_else(|| AppError::BadRequest("session is missing current question text".into()))?;

    let company = super::load_company(&state, &session.company_id).await?;

    let (eval, attempt_n) = evaluations::load_or_create(&state.db, &session, &qid, &qtext).await?;

    let prior_summaries: Vec<String> = summary::recent_company_summaries(
        &state.db,
        &session.company_id,
        Some(&session.id),
        summary::CARRY_FORWARD_INTO_CRITIQUE,
    )
    .await?
    .into_iter()
    .map(|s| s.narrative)
    .collect();

    let critique_ctx = critique::CritiqueCtx {
        db: &state.db,
        book_cache: &state.book_cache,
    };
    let critique = critique::run(
        &critique_ctx,
        &*state.openrouter,
        &session,
        &company,
        &qtext,
        &answer,
        &eval.attempts,
        &prior_summaries,
    )
    .await?;

    // Always attempt to TTS the critique narrative — the audio is then
    // available later (review page, prior attempts) regardless of whether the
    // user wants autoplay. The voice toggle now controls autoplay only.
    let critique_audio_path = match super::tts_to(
        &state,
        &session,
        &super::critique_audio_filename(&qid, attempt_n),
        &critique.narrative,
    )
    .await
    {
        Ok(p) => Some(p),
        Err(e) => {
            tracing::warn!(error = %e, "tts of critique failed; continuing without audio");
            None
        }
    };

    let attempt = Attempt {
        attempt_n,
        answer_audio_path: audio_path,
        answer_transcript: answer,
        critique: Some(critique),
        critique_audio_path,
        created_at: chrono::Utc::now(),
    };
    evaluations::commit_attempt(&state.db, eval, attempt).await?;

    Ok(Redirect::to(&format!("/sessions/{id}")).into_response())
}

pub(super) async fn transcribe(
    State(state): State<AppState>,
    Path(id): Path<String>,
    mut form: Multipart,
) -> Result<Json<serde_json::Value>, AppError> {
    let session = super::load_session(&state, &id).await?;
    if session.status != SessionStatus::Active {
        return Err(AppError::BadRequest("session has ended".into()));
    }

    let mut bytes: Vec<u8> = Vec::new();
    let mut ext: String = "webm".to_string();

    while let Some(field) = form
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart: {e}")))?
    {
        let fname = field.name().unwrap_or("").to_string();
        if fname == "file" || fname == "audio" {
            if let Some(orig) = field.file_name() {
                if let Some(e) = std::path::Path::new(orig)
                    .extension()
                    .and_then(|s| s.to_str())
                {
                    ext = e.to_lowercase();
                }
            }
            bytes = field
                .bytes()
                .await
                .map_err(|e| AppError::BadRequest(format!("audio read: {e}")))?
                .to_vec();
        }
    }

    if bytes.is_empty() {
        return Err(AppError::BadRequest("no audio uploaded".into()));
    }

    let (audio_path, transcript) = stt::save_and_transcribe(
        &*state.openrouter,
        &session.model_snapshot.stt,
        &state.config.data_dir,
        &session.company_id,
        &session.id,
        &bytes,
        &ext,
    )
    .await?;

    Ok(Json(json!({
        "audio_path": audio_path,
        "transcript": transcript,
    })))
}

/// Re-attempt TTS for one specific attempt's critique narrative. Used when
/// the original synthesis failed (transient OpenRouter error) or the audio
/// file got lost. Saves the new MP3 alongside other recordings and updates
/// the eval row's `critique_audio_path` for that attempt.
pub(super) async fn regenerate_critique_audio(
    State(state): State<AppState>,
    Path((id, eval_id, attempt_n)): Path<(String, String, u32)>,
) -> Result<Response, AppError> {
    let session = super::load_session(&state, &id).await?;
    let evals = state.db.collection::<Evaluation>(Evaluation::COLLECTION);
    let eval: Evaluation = evals
        .find_one(bson::doc! { "_id": &eval_id, "session_id": &id })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("evaluation {eval_id}")))?;

    let attempt = eval
        .attempts
        .iter()
        .find(|a| a.attempt_n == attempt_n)
        .ok_or_else(|| AppError::NotFound(format!("attempt {attempt_n} on {eval_id}")))?;

    let critique = attempt
        .critique
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("attempt has no critique to read aloud".into()))?;

    tracing::info!(
        event = "tts.regenerate",
        session_id = %id,
        eval_id = %eval_id,
        attempt_n,
        narrative_chars = critique.narrative.chars().count(),
        "regenerating critique audio",
    );

    let audio_path = super::tts_to(
        &state,
        &session,
        &super::critique_audio_filename(&eval.question_id, attempt_n),
        &critique.narrative,
    )
    .await?;

    // Update the matched array element. MongoDB positional `$` operator finds
    // the right attempt by attempt_n in the array filter.
    evals
        .update_one(
            bson::doc! {
                "_id": &eval_id,
                "attempts.attempt_n": attempt_n as i64,
            },
            bson::doc! { "$set": { "attempts.$.critique_audio_path": &audio_path } },
        )
        .await?;

    tracing::info!(
        event = "tts.regenerate_ok",
        eval_id = %eval_id,
        attempt_n,
        path = %audio_path,
        "critique audio regenerated",
    );

    Ok(Redirect::to(&format!("/sessions/{id}")).into_response())
}

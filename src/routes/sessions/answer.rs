//! Answer flow: submit_answer (record an attempt + critique it), transcribe
//! (STT a recording), regenerate_critique_audio (re-TTS a stored critique).

use axum::extract::{Extension, Form, Path, State};
use axum::response::{IntoResponse, Json, Redirect, Response};
use axum_extra::extract::Multipart;
use serde::Deserialize;
use serde_json::json;

use crate::error::AppError;
use crate::models::{Attempt, SessionStatus};
use crate::services::auth::CurrentUser;
use crate::services::{critique, evaluations, stt, summary, text_validation};
use crate::startup::AppState;

/// Hard cap on a single answer transcript. Generous so long thinking-out-loud
/// answers fit, but rejects pasted-essay-as-answer cases (which would also
/// blow critique tokens).
const MAX_ANSWER_CHARS: usize = 10_000;
/// Per-file cap on uploaded answer audio. The global body-limit is 50 MB to
/// accommodate large PDF assets; audio gets a tighter cap (~5 min @ 64 kbps
/// audio/webm leaves plenty of headroom inside 25 MB).
const MAX_AUDIO_BYTES: usize = 25 * 1024 * 1024;
/// File extensions we will accept as inbound answer audio. Anything outside
/// this set falls back to `webm` (what the browser MediaRecorder default
/// emits), so a client-supplied `original.exe` never lands on disk.
const ALLOWED_AUDIO_EXTS: &[&str] = &["webm", "ogg", "mp3", "wav", "m4a", "mp4"];
const DEFAULT_AUDIO_EXT: &str = "webm";

#[derive(Deserialize)]
pub(super) struct AnswerForm {
    pub transcript: String,
    #[serde(default)]
    pub audio_path: Option<String>,
}

pub(super) async fn submit_answer(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(id): Path<String>,
    Form(form): Form<AnswerForm>,
) -> Result<Response, AppError> {
    let answer = text_validation::sanitize_long(&form.transcript, MAX_ANSWER_CHARS, "answer")?;
    let audio_path = form.audio_path.filter(|s| !s.is_empty());

    let session = super::load_session(&state, &current_user.id, &id).await?;
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

    let company = super::load_company(&state, &current_user.id, &session.company_id).await?;

    let eval_source = evaluations::PgEvalSource { pool: &state.pool };
    let (eval, attempt_n) =
        evaluations::load_or_create(&eval_source, &session, &qid, &qtext).await?;

    let prior_summaries: Vec<String> = summary::recent_company_summaries(
        &state.pool,
        &current_user.id,
        &session.company_id,
        Some(&session.id),
        summary::CARRY_FORWARD_INTO_CRITIQUE,
    )
    .await?
    .into_iter()
    .map(|s| s.narrative)
    .collect();

    let critique_ctx = critique::CritiqueCtx {
        pool: &state.pool,
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
        super::TtsConfig::from_snapshot(&session.model_snapshot),
    )
    .await
    {
        Ok(p) => Some(p),
        Err(e) => {
            tracing::warn!(
                error = %e,
                session_id = %session.id,
                question_id = %qid,
                attempt_n,
                "tts of critique failed; continuing without audio",
            );
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
    evaluations::commit_attempt(&eval_source, eval, attempt).await?;

    Ok(Redirect::to(&format!("/sessions/{id}")).into_response())
}

/// Normalize a client-supplied audio extension. Lowercased + checked
/// against [`ALLOWED_AUDIO_EXTS`]; anything outside the list falls back
/// to [`DEFAULT_AUDIO_EXT`] so the on-disk filename can never carry an
/// attacker-chosen suffix.
fn sanitize_audio_ext(ext: &str) -> String {
    let lower = ext.to_lowercase();
    if ALLOWED_AUDIO_EXTS.iter().any(|a| *a == lower) {
        lower
    } else {
        DEFAULT_AUDIO_EXT.to_string()
    }
}

pub(super) async fn transcribe(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(id): Path<String>,
    mut form: Multipart,
) -> Result<Json<serde_json::Value>, AppError> {
    let session = super::load_session(&state, &current_user.id, &id).await?;
    if session.status != SessionStatus::Active {
        return Err(AppError::BadRequest("session has ended".into()));
    }

    let mut bytes: Vec<u8> = Vec::new();
    let mut ext: String = DEFAULT_AUDIO_EXT.to_string();

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
                    ext = sanitize_audio_ext(e);
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
    if bytes.len() > MAX_AUDIO_BYTES {
        return Err(AppError::BadRequest(format!(
            "audio is {} bytes — max {MAX_AUDIO_BYTES}",
            bytes.len()
        )));
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
    Extension(current_user): Extension<CurrentUser>,
    Path((id, eval_id, attempt_n)): Path<(String, String, u32)>,
) -> Result<Response, AppError> {
    let session = super::load_session(&state, &current_user.id, &id).await?;
    let mut eval =
        evaluations::load_for_attempt_edit(&state.pool, &current_user.id, &id, &eval_id).await?;

    let critique = eval
        .attempts
        .iter()
        .find(|a| a.attempt_n == attempt_n)
        .ok_or_else(|| AppError::NotFound(format!("attempt {attempt_n} on {eval_id}")))?
        .critique
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("attempt has no critique to read aloud".into()))?
        .clone();

    tracing::info!(
        event = "tts.regenerate",
        session_id = %id,
        eval_id = %eval_id,
        attempt_n,
        narrative_chars = critique.narrative.chars().count(),
        "regenerating critique audio",
    );

    // Regenerate intentionally uses *current* settings rather than the
    // session's frozen model_snapshot. The snapshot is what produced the
    // failed audio in the first place — re-using it would just repeat
    // the same upstream error after a TTS model change in /settings.
    let settings = state.settings_cache.get(&state.pool).await?;
    let audio_path = super::tts_to(
        &state,
        &session,
        &super::critique_audio_filename(&eval.question_id, attempt_n),
        &critique.narrative,
        super::TtsConfig::from_settings(&settings),
    )
    .await?;

    // Mutate the matched attempt in place + persist via the EvalSource
    // upsert. The full row roundtrip is cheaper than a JSONB-position update
    // and stays inside the same trait surface tests already use.
    for a in eval.attempts.iter_mut() {
        if a.attempt_n == attempt_n {
            a.critique_audio_path = Some(audio_path.clone());
            break;
        }
    }
    let eval_source = evaluations::PgEvalSource { pool: &state.pool };
    use crate::services::evaluations::EvalSource;
    eval_source.upsert(&eval).await?;

    tracing::info!(
        event = "tts.regenerate_ok",
        eval_id = %eval_id,
        attempt_n,
        path = %audio_path,
        "critique audio regenerated",
    );

    Ok(Redirect::to(&format!("/sessions/{id}")).into_response())
}

#[cfg(test)]
#[path = "answer_tests.rs"]
mod tests;

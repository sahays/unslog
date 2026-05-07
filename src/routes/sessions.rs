use askama::Template;
use axum::extract::{Form, Path, State};
use axum::response::{Html, IntoResponse, Json, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use axum_extra::extract::Multipart;
use futures::TryStreamExt;
use mongodb::options::FindOptions;
use serde::Deserialize;
use serde_json::json;

use crate::error::AppError;
use crate::models::{
    Attempt, Company, Evaluation, ModelSnapshot, PromptSnapshot, Session, SessionStatus, Summary,
};
use crate::services::{critique, openrouter, prompt_store, question_bank, stt, summary, tts};
use crate::startup::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/companies/:id/sessions", post(start))
        .route("/sessions/:id", get(show))
        .route("/sessions/:id/next-question", post(next_question))
        .route("/sessions/:id/transcribe", post(transcribe))
        .route("/sessions/:id/answers", post(submit_answer))
        .route("/sessions/:id/toggle-voice", post(toggle_voice))
        .route("/sessions/:id/end", post(end))
}

async fn start(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let coll = crate::db::companies(&state.db);
    let _company: Company = coll
        .find_one(bson::doc! { "_id": &id })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("company {id}")))?;

    let critique_prompt = prompt_store::get_prompt(&state.db, "critique")
        .await?
        .ok_or_else(|| AppError::NotFound("critique prompt".into()))?;
    let summary_prompt = prompt_store::get_prompt(&state.db, "summary")
        .await?
        .ok_or_else(|| AppError::NotFound("summary prompt".into()))?;

    let session = Session {
        id: uuid::Uuid::now_v7().to_string(),
        company_id: id,
        started_at: chrono::Utc::now(),
        ended_at: None,
        status: SessionStatus::Active,
        model_snapshot: ModelSnapshot {
            stt: openrouter::DEFAULT_STT_MODEL.into(),
            tts: openrouter::DEFAULT_TTS_MODEL.into(),
            critique: openrouter::DEFAULT_CRITIQUE_MODEL.into(),
            research: openrouter::DEFAULT_RESEARCH_MODEL.into(),
        },
        prompt_snapshot: PromptSnapshot {
            critique: critique_prompt.current_version_id,
            summary: summary_prompt.current_version_id,
        },
        voice_critique_enabled: false,
        current_question_id: None,
        current_question_text: None,
        current_question_audio_path: None,
    };

    state
        .db
        .collection::<Session>(Session::COLLECTION)
        .insert_one(&session)
        .await?;

    Ok(Redirect::to(&format!("/sessions/{}", session.id)).into_response())
}

#[derive(Template)]
#[template(path = "sessions/active.html")]
struct ActiveTemplate {
    session: Session,
    company: Company,
    history: Vec<Evaluation>,
    current_eval: Option<Evaluation>,
    has_primary_asset: bool,
    summary: Option<Summary>,
}

async fn show(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Html<String>, AppError> {
    let session = load_session(&state, &id).await?;
    let company = load_company(&state, &session.company_id).await?;

    let evals: Vec<Evaluation> = state
        .db
        .collection::<Evaluation>(Evaluation::COLLECTION)
        .find(bson::doc! { "session_id": &id })
        .with_options(FindOptions::builder().sort(bson::doc! { "_id": 1 }).build())
        .await?
        .try_collect()
        .await?;

    let current_eval = match &session.current_question_id {
        Some(qid) => evals.iter().find(|e| &e.question_id == qid).cloned(),
        None => None,
    };
    let history: Vec<Evaluation> = evals
        .into_iter()
        .filter(|e| Some(&e.question_id) != session.current_question_id.as_ref())
        .collect();

    let has_primary_asset = crate::db::assets(&state.db)
        .count_documents(bson::doc! { "primary": true })
        .await?
        > 0;

    let summary = summary::for_session(&state.db, &id).await?;

    let body = ActiveTemplate {
        session,
        company,
        history,
        current_eval,
        has_primary_asset,
        summary,
    }
    .render()
    .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(body))
}

async fn next_question(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let session = load_session(&state, &id).await?;
    if session.status != SessionStatus::Active {
        return Err(AppError::BadRequest("session has ended".into()));
    }

    let bank = question_bank::ensure_for(&state.db, &session.company_id).await?;
    let evals: Vec<Evaluation> = state
        .db
        .collection::<Evaluation>(Evaluation::COLLECTION)
        .find(bson::doc! { "session_id": &id })
        .await?
        .try_collect()
        .await?;
    let seen: Vec<String> = evals.iter().map(|e| e.question_id.clone()).collect();

    let Some(next) = question_bank::pick_next(&bank, &seen) else {
        return Err(AppError::BadRequest(
            "no unseen questions left in this bank".into(),
        ));
    };

    // Best-effort TTS of the question text. If TTS fails (no key, model error,
    // network), keep the question but skip the audio — the page still works.
    let audio_path = match maybe_tts_question(&state, &session, next).await {
        Ok(p) => Some(p),
        Err(e) => {
            tracing::warn!(error = %e, "tts of question failed; continuing without audio");
            None
        }
    };

    state
        .db
        .collection::<Session>(Session::COLLECTION)
        .update_one(
            bson::doc! { "_id": &id },
            bson::doc! { "$set": {
                "current_question_id": &next.id,
                "current_question_text": &next.text,
                "current_question_audio_path": audio_path.clone(),
            } },
        )
        .await?;

    Ok(Redirect::to(&format!("/sessions/{id}")).into_response())
}

async fn maybe_tts_question(
    state: &AppState,
    session: &Session,
    question: &crate::models::Question,
) -> Result<String, AppError> {
    if !state.openrouter.configured() {
        return Err(AppError::OpenRouterNotConfigured);
    }
    let dir = crate::recordings::session_dir(
        &state.config.data_dir,
        &session.company_id,
        &session.id,
    );
    crate::recordings::ensure_dir(&dir).await?;
    let path = dir.join(format!("question_{}.mp3", question.id));
    let path = tts::synthesize(
        &state.openrouter,
        &session.model_snapshot.tts,
        openrouter::DEFAULT_TTS_VOICE,
        &question.text,
        path,
    )
    .await?;
    Ok(path.to_string_lossy().into_owned())
}

#[derive(Deserialize)]
pub struct AnswerForm {
    pub transcript: String,
    #[serde(default)]
    pub audio_path: Option<String>,
}

async fn submit_answer(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Form(form): Form<AnswerForm>,
) -> Result<Response, AppError> {
    let answer = form.transcript.trim().to_string();
    if answer.is_empty() {
        return Err(AppError::BadRequest("answer is empty".into()));
    }
    let audio_path = form.audio_path.filter(|s| !s.is_empty());

    let session = load_session(&state, &id).await?;
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

    let company = load_company(&state, &session.company_id).await?;

    let evals = state.db.collection::<Evaluation>(Evaluation::COLLECTION);
    let mut eval = evals
        .find_one(bson::doc! { "session_id": &id, "question_id": &qid })
        .await?
        .unwrap_or_else(|| {
            Evaluation::new(
                session.id.clone(),
                session.company_id.clone(),
                qid.clone(),
                qtext.clone(),
            )
        });

    let attempt_n = (eval.attempts.len() as u32) + 1;

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

    let critique = critique::run(
        &state.openrouter,
        &state.db,
        &session,
        &company,
        &qtext,
        &answer,
        &eval.attempts,
        &prior_summaries,
    )
    .await?;

    let critique_audio_path = if session.voice_critique_enabled {
        match maybe_tts_critique(&state, &session, &critique.narrative, attempt_n, &qid).await {
            Ok(p) => Some(p),
            Err(e) => {
                tracing::warn!(error = %e, "tts of critique failed");
                None
            }
        }
    } else {
        None
    };

    eval.attempts.push(Attempt {
        attempt_n,
        answer_audio_path: audio_path,
        answer_transcript: answer,
        critique: Some(critique),
        critique_audio_path,
        created_at: chrono::Utc::now(),
    });

    if eval.attempts.len() == 1 {
        evals.insert_one(&eval).await?;
    } else {
        evals
            .replace_one(bson::doc! { "_id": &eval.id }, &eval)
            .await?;
    }

    Ok(Redirect::to(&format!("/sessions/{id}")).into_response())
}

async fn transcribe(
    State(state): State<AppState>,
    Path(id): Path<String>,
    mut form: Multipart,
) -> Result<Json<serde_json::Value>, AppError> {
    let session = load_session(&state, &id).await?;
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
                if let Some(e) = std::path::Path::new(orig).extension().and_then(|s| s.to_str()) {
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
        &state.openrouter,
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

async fn maybe_tts_critique(
    state: &AppState,
    session: &Session,
    text: &str,
    attempt_n: u32,
    question_id: &str,
) -> Result<String, AppError> {
    if !state.openrouter.configured() {
        return Err(AppError::OpenRouterNotConfigured);
    }
    let dir = crate::recordings::session_dir(
        &state.config.data_dir,
        &session.company_id,
        &session.id,
    );
    crate::recordings::ensure_dir(&dir).await?;
    let path = dir.join(format!("critique_{question_id}_v{attempt_n}.mp3"));
    let path = tts::synthesize(
        &state.openrouter,
        &session.model_snapshot.tts,
        openrouter::DEFAULT_TTS_VOICE,
        text,
        path,
    )
    .await?;
    Ok(path.to_string_lossy().into_owned())
}

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

async fn end(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let session = load_session(&state, &id).await?;
    if session.status == SessionStatus::Active {
        let company = load_company(&state, &session.company_id).await?;
        // Best-effort: if the LLM call fails (no key, model down), still let
        // the session end so the user isn't stuck with an unkillable session.
        if let Err(e) = summary::generate_and_save(&state.openrouter, &state.db, &session, &company).await {
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
                "ended_at": bson::DateTime::now(),
                "current_question_id": null,
                "current_question_text": null,
            } },
        )
        .await?;
    Ok(Redirect::to(&format!("/sessions/{id}")).into_response())
}

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

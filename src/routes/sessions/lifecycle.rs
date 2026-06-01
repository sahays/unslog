//! Session lifecycle routes: start / show / review / next-question, plus the
//! `advance_to_next` engine that picks the next curated question and TTSes
//! the prompt audio.

use askama::Template;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use futures::TryStreamExt;
use mongodb::options::FindOptions;

use crate::error::AppError;
use crate::filters; // Custom Askama filters used by the templates below.
use crate::models::{Company, Evaluation, Session, SessionStatus, Summary};
use crate::services::{asset_store, company_store, evaluations, questions, sessions, summary};
use crate::startup::AppState;

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

#[derive(Template)]
#[template(path = "sessions/review.html")]
struct ReviewTemplate {
    session: Session,
    company: Company,
    evals: Vec<Evaluation>,
    summary: Option<Summary>,
}

pub(super) async fn start(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let company = company_store::find_or_404(&state.db, &id).await?;

    // Single-company entry: scope = just this company.
    let session = crate::services::session::start(
        &state.db,
        &state.pool,
        &*state.openrouter,
        crate::services::session::StartInput {
            role: company.canonical_role,
            anchor_company_id: id,
            selected_company_ids: vec![company.id.clone()],
        },
    )
    .await?;

    tracing::info!(
        event = "session.start",
        kind = "single_company",
        session_id = %session.id,
        company_id = %session.company_id,
        role = session.role.as_str(),
        critique_model = %session.model_snapshot.critique,
        research_model = %session.model_snapshot.research,
        curated_count = session.curated_question_ids.len(),
        "session started",
    );

    if let Err(e) = advance_to_next(&state, &session).await {
        tracing::warn!(error = %e, session_id = %session.id, "failed to auto-load first question; user can pick manually");
    }

    Ok(Redirect::to(&format!("/sessions/{}", session.id)).into_response())
}

pub(super) async fn show(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Html<String>, AppError> {
    let session = super::load_session(&state, &id).await?;
    let company = super::load_company(&state, &session.company_id).await?;

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

    // Critique requires a primary *book*. Resume presence is unrelated;
    // scope the check to AssetKind::Book.
    let has_primary_asset =
        asset_store::find_primary_by_kind(&state.db, crate::models::AssetKind::Book)
            .await?
            .is_some();

    let summary = summary::for_session(&state.db, &id).await?;

    crate::error::render_html(ActiveTemplate {
        session,
        company,
        history,
        current_eval,
        has_primary_asset,
        summary,
    })
}

pub(super) async fn review(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Html<String>, AppError> {
    let session = super::load_session(&state, &id).await?;
    let company = super::load_company(&state, &session.company_id).await?;
    let evals: Vec<Evaluation> = state
        .db
        .collection::<Evaluation>(Evaluation::COLLECTION)
        .find(bson::doc! { "session_id": &id })
        .with_options(FindOptions::builder().sort(bson::doc! { "_id": 1 }).build())
        .await?
        .try_collect()
        .await?;
    let summary = summary::for_session(&state.db, &id).await?;
    crate::error::render_html(ReviewTemplate {
        session,
        company,
        evals,
        summary,
    })
}

pub(super) async fn next_question(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let session = super::load_session(&state, &id).await?;
    if session.status != SessionStatus::Active {
        return Err(AppError::BadRequest("session has ended".into()));
    }

    match advance_to_next(&state, &session).await? {
        Some(()) => Ok(Redirect::to(&format!("/sessions/{id}")).into_response()),
        None => end_inline(&state, &session).await,
    }
}

/// Pick the next question for an active session and persist it as
/// `current_question_*`. Returns `Some(())` if a question was loaded;
/// `None` if there is nothing to load (curated list exhausted, or no
/// fallback questions exist). Used by both the next-question button and
/// the session-start handlers — landing on a fresh session shouldn't
/// require an extra click to see the first question.
pub(crate) async fn advance_to_next(
    state: &AppState,
    session: &Session,
) -> Result<Option<()>, AppError> {
    // Only the question_id set is needed here — narrow projection skips
    // the embedded `attempts[]` payload on every Evaluation row.
    let answered = evaluations::answered_question_ids(&state.db, &session.id).await?;

    let next_id = crate::services::curator::next_curated(session, &answered);

    let pair = if let Some(next_id) = next_id {
        load_question_by_id(state, session, &next_id).await?
    } else if session.curated_question_ids.is_empty() {
        // Legacy fallback: random from the company's questions.
        let pool = questions::list_for_company(&state.db, &session.company_id).await?;
        questions::pick_random(&pool, &answered).map(|q| (q.id.clone(), q.text.clone()))
    } else {
        return Ok(None);
    };

    let Some((qid, qtext)) = pair else {
        return Ok(None);
    };

    let audio_path =
        match super::tts_to(state, session, &format!("question_{qid}.mp3"), &qtext).await {
            Ok(p) => Some(p),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    session_id = %session.id,
                    question_id = %qid,
                    "tts of question failed; continuing without audio",
                );
                None
            }
        };

    sessions::set_current_question(&state.db, &session.id, &qid, &qtext, audio_path.as_deref())
        .await?;

    Ok(Some(()))
}

/// Look up a question's text by its ID. With a top-level Questions
/// collection this is a direct lookup — no need to walk per-company banks.
async fn load_question_by_id(
    state: &AppState,
    _session: &Session,
    qid: &str,
) -> Result<Option<(String, String)>, AppError> {
    let q = questions::get(&state.db, qid).await?;
    Ok(q.map(|q| (q.id, q.text)))
}

/// Fire the existing `end` flow inline (used when curated list is exhausted).
async fn end_inline(state: &AppState, session: &Session) -> Result<Response, AppError> {
    let session_id = session.id.clone();
    let company = super::load_company(state, &session.company_id).await?;
    let summary_ctx = summary::SummaryCtx {
        db: &state.db,
        pool: &state.pool,
    };
    if let Err(e) =
        summary::generate_and_save(&summary_ctx, &*state.openrouter, session, &company).await
    {
        tracing::warn!(error = %e, session_id = %session_id, "summary generation failed at auto-end");
    }
    sessions::end(&state.db, &session_id).await?;
    tracing::info!(
        event = "session.auto_end",
        session_id = %session_id,
        "session auto-ended after curated questions exhausted",
    );
    Ok(Redirect::to(&format!("/sessions/{session_id}")).into_response())
}

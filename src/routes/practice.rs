//! `/practice` — top-level cross-company session entry point.
//! Pick role + selected companies → curator picks 4–6 questions → session starts.

use askama::Template;
use axum::extract::State;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use axum_extra::extract::Form;
use futures::TryStreamExt;
use mongodb::options::FindOptions;
use serde::Deserialize;

use std::collections::HashMap;

use crate::error::AppError;
use crate::models::{
    Company, Evaluation, ModelSnapshot, PromptSnapshot, Role, Session, SessionStatus,
};
use crate::services::{curator, prompt_store, settings_store};
use crate::startup::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/practice", get(show).post(start))
}

#[derive(Template)]
#[template(path = "practice/index.html")]
struct PracticeTemplate {
    role_options: Vec<(&'static str, &'static str)>,
    companies: Vec<Company>,
    in_progress: Vec<InProgress>,
}

pub struct InProgress {
    pub session_id: String,
    pub started_at: String,
    pub role_label: String,
    pub anchor_company: String,
    pub companies_n: usize,
    pub answered: usize,
    pub total: usize,
    /// True when the anchor company has been deleted — opening the session
    /// would 404, so the UI offers Delete only.
    pub stale: bool,
}

async fn show(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let opts = FindOptions::builder()
        .sort(bson::doc! { "name": 1 })
        .build();
    let cursor = crate::db::companies(&state.db)
        .find(bson::doc! {})
        .with_options(opts)
        .await?;
    let companies: Vec<Company> = cursor.try_collect().await?;
    let company_by_id: HashMap<String, &Company> =
        companies.iter().map(|c| (c.id.clone(), c)).collect();

    let active_opts = FindOptions::builder()
        .sort(bson::doc! { "started_at": -1 })
        .build();
    let active_cursor = state
        .db
        .collection::<Session>(Session::COLLECTION)
        .find(bson::doc! { "status": "active" })
        .with_options(active_opts)
        .await?;
    let active_sessions: Vec<Session> = active_cursor.try_collect().await?;

    let mut in_progress: Vec<InProgress> = Vec::with_capacity(active_sessions.len());
    for s in &active_sessions {
        let answered_n = state
            .db
            .collection::<Evaluation>(Evaluation::COLLECTION)
            .count_documents(bson::doc! { "session_id": &s.id })
            .await? as usize;

        let total = if s.curated_question_ids.is_empty() {
            answered_n
        } else {
            s.curated_question_ids.len()
        };

        let anchor_lookup = company_by_id.get(&s.company_id);
        let anchor = anchor_lookup
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "Deleted company".to_string());
        let stale = anchor_lookup.is_none();

        in_progress.push(InProgress {
            session_id: s.id.clone(),
            started_at: s.started_at.format("%b %d, %H:%M").to_string(),
            role_label: s.role.display_name().to_string(),
            anchor_company: anchor,
            companies_n: s.selected_company_ids.len().max(1),
            answered: answered_n,
            total,
            stale,
        });
    }

    let role_options = Role::ALL
        .iter()
        .map(|r| (r.as_str(), r.display_name()))
        .collect();

    let body = PracticeTemplate {
        role_options,
        companies,
        in_progress,
    }
    .render()
    .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(body))
}

#[derive(Deserialize)]
pub struct StartForm {
    pub role: String,
    /// Checkbox group — repeated `companies=<id>` keys. `axum_extra::Form`
    /// (serde_html_form) collects repeated keys into a Vec; the stock
    /// `axum::Form` (serde_urlencoded) only keeps the last value.
    #[serde(default)]
    pub companies: Vec<String>,
}

async fn start(
    State(state): State<AppState>,
    Form(form): Form<StartForm>,
) -> Result<Response, AppError> {
    let role = Role::parse(form.role.trim())
        .ok_or_else(|| AppError::BadRequest(format!("unknown role {}", form.role)))?;
    let mut selected: Vec<String> = form
        .companies
        .into_iter()
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect();
    selected.sort();
    selected.dedup();

    if selected.is_empty() {
        return Err(AppError::BadRequest(
            "pick at least one company to draw questions from".into(),
        ));
    }

    // Validate the picked companies exist and match the chosen role.
    let coll = crate::db::companies(&state.db);
    let mut matched: Vec<Company> = Vec::with_capacity(selected.len());
    for cid in &selected {
        match coll.find_one(bson::doc! { "_id": cid }).await? {
            Some(c) if c.canonical_role == role => matched.push(c),
            Some(_) => {
                tracing::warn!(
                    company_id = %cid,
                    role = role.as_str(),
                    "company picked for /practice but its canonical_role doesn't match — skipping",
                );
            }
            None => {
                return Err(AppError::NotFound(format!("company {cid}")));
            }
        }
    }
    if matched.is_empty() {
        return Err(AppError::BadRequest(format!(
            "no selected companies match role {}",
            role.display_name()
        )));
    }

    let critique_prompt = prompt_store::get_prompt(&state.db, "critique")
        .await?
        .ok_or_else(|| AppError::NotFound("critique prompt".into()))?;
    let summary_prompt = prompt_store::get_prompt(&state.db, "summary")
        .await?
        .ok_or_else(|| AppError::NotFound("summary prompt".into()))?;
    let settings = settings_store::load(&state.db).await?;

    let selected_company_ids: Vec<String> = matched.iter().map(|c| c.id.clone()).collect();
    let curated = curator::curate(
        &state.openrouter,
        &state.db,
        &settings.lite_model,
        role,
        &selected_company_ids,
    )
    .await?;

    // Anchor company = first selected. The critique still uses the *source*
    // question's company packet (handled in critique service), but the
    // session has a company_id field for reverse-lookup.
    let anchor = matched[0].clone();

    let session = Session {
        id: uuid::Uuid::now_v7().to_string(),
        company_id: anchor.id.clone(),
        role,
        selected_company_ids,
        curated_question_ids: curated.question_ids.clone(),
        focus_line: curated.focus_line.clone(),
        started_at: chrono::Utc::now(),
        ended_at: None,
        status: SessionStatus::Active,
        model_snapshot: ModelSnapshot {
            stt: settings.stt_model.clone(),
            tts: settings.tts_model.clone(),
            critique: settings.critique_model.clone(),
            research: settings.research_model.clone(),
            tts_voice: settings.tts_voice.clone(),
            tts_speed: settings.tts_speed,
            lite: settings.lite_model.clone(),
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
    tracing::info!(
        event = "session.start",
        kind = "cross_company",
        session_id = %session.id,
        role = role.as_str(),
        companies_n = matched.len(),
        curated_count = session.curated_question_ids.len(),
        "cross-company session started",
    );

    if let Err(e) = crate::routes::sessions::advance_to_next(&state, &session).await {
        tracing::warn!(error = %e, session_id = %session.id, "failed to auto-load first question; user can pick manually");
    }

    Ok(Redirect::to(&format!("/sessions/{}", session.id)).into_response())
}

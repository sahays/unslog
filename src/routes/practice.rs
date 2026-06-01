//! `/practice` — top-level cross-company session entry point.
//! Pick role + selected companies → curator picks 4–6 questions → session starts.

use askama::Template;
use axum::extract::State;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use axum_extra::extract::Form;
use serde::Deserialize;

use std::collections::HashMap;

use crate::error::AppError;
use crate::models::{Company, Role};
use crate::services::{company_store, evaluations, sessions};
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
    let companies = company_store::list_by_name(&state.pool).await?;
    let company_by_id: HashMap<String, &Company> =
        companies.iter().map(|c| (c.id.clone(), c)).collect();

    let active_sessions = sessions::list_active(&state.pool).await?;

    // Bulk eval counts: one aggregate keyed by session_id instead of N
    // count_documents calls.
    let session_ids: Vec<&str> = active_sessions.iter().map(|s| s.id.as_str()).collect();
    let answered_by_session = evaluations::counts_by_session(&state.pool, &session_ids).await?;

    let mut in_progress: Vec<InProgress> = Vec::with_capacity(active_sessions.len());
    for s in &active_sessions {
        let answered_n = answered_by_session.get(&s.id).copied().unwrap_or(0);

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

    crate::error::render_html(PracticeTemplate {
        role_options,
        companies,
        in_progress,
    })
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

    // Bulk-load the picked companies in one `$in` query, then validate
    // existence + role in app code. Avoids N round-trips for N selected
    // companies (no big-O improvement for tiny N, but the contract is the
    // same and the index hit is cheaper).
    let rows = company_store::find_by_ids(&state.pool, &selected).await?;
    let mut by_id: HashMap<String, Company> = rows.into_iter().map(|c| (c.id.clone(), c)).collect();
    let mut matched: Vec<Company> = Vec::with_capacity(selected.len());
    for cid in &selected {
        match by_id.remove(cid) {
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

    // Anchor company = first selected. The critique still uses the *source*
    // question's company packet (handled in critique service), but the
    // session has a company_id field for reverse-lookup.
    let selected_company_ids: Vec<String> = matched.iter().map(|c| c.id.clone()).collect();
    let anchor_id = matched[0].id.clone();

    let session = crate::services::session::start(
        &state.pool,
        &*state.openrouter,
        crate::services::session::StartInput {
            role,
            anchor_company_id: anchor_id,
            selected_company_ids,
        },
    )
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

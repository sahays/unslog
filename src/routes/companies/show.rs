//! `/companies/:id` show page — company detail, question bank, sessions.

use std::collections::HashMap;

use askama::Template;
use axum::extract::{Path, State};
use axum::response::Html;
use futures::TryStreamExt;
use serde::Deserialize;

use crate::error::AppError;
use crate::filters; // Custom Askama filters used by templates below.
use crate::models::{Company, Question, Summary};
use crate::services::{category_store, evaluations, questions, sessions as session_store};
use crate::startup::AppState;

use super::SessionRow;

#[derive(Template)]
#[template(path = "companies/show.html")]
struct ShowTemplate {
    company: Company,
    questions: Vec<Question>,
    sessions: Vec<SessionRow>,
    canonical_categories: Vec<crate::models::Category>,
    role_options: Vec<(&'static str, &'static str)>,
    edit_qid: Option<String>,
}

#[derive(Deserialize)]
pub struct ShowQuery {
    #[serde(default)]
    pub edit: Option<String>,
}

pub async fn show(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<ShowQuery>,
) -> Result<Html<String>, AppError> {
    let company = super::load_company(&state, &id).await?;
    let questions_list = questions::list_for_company(&state.db, &id).await?;
    let sessions = load_session_rows(&state, &id).await?;
    let canonical_categories = category_store::list_all(&state.db).await?;

    crate::error::render_html(ShowTemplate {
        company,
        questions: questions_list,
        sessions,
        canonical_categories,
        role_options: super::role_options(),
        edit_qid: query.edit,
    })
}

/// Load sessions for a company plus their eval counts and summaries in two
/// bulk queries (instead of 2× per session). Sorted newest-first.
async fn load_session_rows(
    state: &AppState,
    company_id: &str,
) -> Result<Vec<SessionRow>, AppError> {
    let sessions_raw = session_store::list_for_company(&state.db, company_id).await?;

    let session_ids: Vec<&str> = sessions_raw.iter().map(|s| s.id.as_str()).collect();
    let counts = evaluations::counts_by_session(&state.db, &session_ids).await?;
    let summary_by_session = load_summaries_by_session(state, &session_ids).await?;

    Ok(sessions_raw
        .into_iter()
        .map(|s| SessionRow {
            eval_count: counts.get(&s.id).copied().unwrap_or(0) as u64,
            summary: summary_by_session.get(&s.id).cloned(),
            session: s,
        })
        .collect())
}

async fn load_summaries_by_session(
    state: &AppState,
    session_ids: &[&str],
) -> Result<HashMap<String, Summary>, AppError> {
    if session_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let summaries: Vec<Summary> = state
        .db
        .collection::<Summary>(Summary::COLLECTION)
        .find(bson::doc! { "session_id": { "$in": session_ids } })
        .await?
        .try_collect()
        .await?;
    Ok(summaries
        .into_iter()
        .map(|s| (s.session_id.clone(), s))
        .collect())
}

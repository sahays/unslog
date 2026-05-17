use std::collections::{HashMap, HashSet};

use askama::Template;
use axum::{extract::State, response::Html, routing::get, Router};
use futures::TryStreamExt;
use mongodb::options::FindOptions;

use crate::error::AppError;
use crate::filters; // Custom Askama filters used by templates below.
use crate::models::{Company, Session, Summary};
use crate::startup::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(index))
}

pub struct RecentRow {
    pub session: Session,
    pub company_name: String,
    pub has_summary: bool,
}

#[derive(Template)]
#[template(path = "home.html")]
struct HomeTemplate {
    company_count: u64,
    primary_asset_present: bool,
    openrouter_configured: bool,
    active_count: u64,
    recent: Vec<RecentRow>,
}

async fn index(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let company_count = crate::db::companies(&state.db)
        .count_documents(bson::doc! {})
        .await?;
    let primary_asset_present = crate::db::assets(&state.db)
        .count_documents(bson::doc! { "primary": true })
        .await?
        > 0;

    let sessions_coll = state.db.collection::<Session>(Session::COLLECTION);
    let active_count = sessions_coll
        .count_documents(bson::doc! { "status": "active" })
        .await?;
    let opts = FindOptions::builder()
        .sort(bson::doc! { "started_at": -1 })
        .limit(5)
        .build();
    let recent_sessions: Vec<Session> = sessions_coll
        .find(bson::doc! {})
        .with_options(opts)
        .await?
        .try_collect()
        .await?;

    // Bulk-load company names and summary presence in two queries instead of
    // 2× per session.
    let company_ids: Vec<&str> = recent_sessions
        .iter()
        .map(|s| s.company_id.as_str())
        .collect();
    let session_ids: Vec<&str> = recent_sessions.iter().map(|s| s.id.as_str()).collect();
    let companies: Vec<Company> = if company_ids.is_empty() {
        Vec::new()
    } else {
        crate::db::companies(&state.db)
            .find(bson::doc! { "_id": { "$in": &company_ids } })
            .await?
            .try_collect()
            .await?
    };
    let company_by_id: HashMap<String, String> =
        companies.into_iter().map(|c| (c.id, c.name)).collect();
    let sessions_with_summary: HashSet<String> = if session_ids.is_empty() {
        HashSet::new()
    } else {
        let summaries: Vec<Summary> = state
            .db
            .collection::<Summary>(Summary::COLLECTION)
            .find(bson::doc! { "session_id": { "$in": &session_ids } })
            .await?
            .try_collect()
            .await?;
        summaries.into_iter().map(|s| s.session_id).collect()
    };

    let recent: Vec<RecentRow> = recent_sessions
        .into_iter()
        .map(|s| {
            let company_name = company_by_id
                .get(&s.company_id)
                .cloned()
                .unwrap_or_else(|| "(deleted company)".into());
            let has_summary = sessions_with_summary.contains(&s.id);
            RecentRow {
                session: s,
                company_name,
                has_summary,
            }
        })
        .collect();

    let body = HomeTemplate {
        company_count,
        primary_asset_present,
        openrouter_configured: state.openrouter.configured(),
        active_count,
        recent,
    }
    .render()
    .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(body))
}

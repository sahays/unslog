use askama::Template;
use axum::{extract::State, response::Html, routing::get, Router};
use futures::TryStreamExt;
use mongodb::options::FindOptions;

use crate::error::AppError;
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

    let mut recent = Vec::with_capacity(recent_sessions.len());
    for s in recent_sessions {
        let company_name = crate::db::companies(&state.db)
            .find_one(bson::doc! { "_id": &s.company_id })
            .await?
            .map(|c: Company| c.name)
            .unwrap_or_else(|| "(deleted company)".into());
        let has_summary = state
            .db
            .collection::<Summary>(Summary::COLLECTION)
            .count_documents(bson::doc! { "session_id": &s.id })
            .await?
            > 0;
        recent.push(RecentRow {
            session: s,
            company_name,
            has_summary,
        });
    }

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

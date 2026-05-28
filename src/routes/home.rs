use std::collections::HashSet;

use askama::Template;
use axum::{extract::State, response::Html, routing::get, Router};
use futures::TryStreamExt;

use crate::error::AppError;
use crate::filters; // Custom Askama filters used by templates below.
use crate::models::{Session, Summary};
use crate::services::{asset_store, company_store, sessions};
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
    /// Pre-computed badge copy for the Setup cards. Pre-computing here keeps
    /// the template logic-free since Askama 0.12 doesn't support
    /// `if`-expressions inside `{% let %}`.
    openrouter_badge_text: &'static str,
    openrouter_badge_variant: &'static str,
    asset_badge_text: &'static str,
    asset_badge_variant: &'static str,
    active_count: u64,
    recent: Vec<RecentRow>,
}

async fn index(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let company_count = company_store::count(&state.db).await?;
    // Critique flow requires a primary *book*. A resume alone doesn't
    // unblock it; scope the presence check to AssetKind::Book.
    let primary_asset_present =
        asset_store::find_primary_by_kind(&state.db, crate::models::AssetKind::Book)
            .await?
            .is_some();

    let active_count = sessions::count_active(&state.db).await?;
    let recent_sessions = sessions::list_recent(&state.db, 5).await?;

    // Bulk-load company names and summary presence in two queries instead of
    // 2× per session.
    let company_ids: Vec<&str> = recent_sessions
        .iter()
        .map(|s| s.company_id.as_str())
        .collect();
    let session_ids: Vec<&str> = recent_sessions.iter().map(|s| s.id.as_str()).collect();
    let company_by_id = company_store::names_by_ids(&state.db, &company_ids).await?;
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

    let openrouter_configured = state.openrouter.configured();
    crate::error::render_html(HomeTemplate {
        company_count,
        primary_asset_present,
        openrouter_configured,
        openrouter_badge_text: if openrouter_configured {
            "key set"
        } else {
            "no key"
        },
        openrouter_badge_variant: if openrouter_configured { "good" } else { "bad" },
        asset_badge_text: if primary_asset_present {
            "configured"
        } else {
            "missing"
        },
        asset_badge_variant: if primary_asset_present { "good" } else { "bad" },
        active_count,
        recent,
    })
}

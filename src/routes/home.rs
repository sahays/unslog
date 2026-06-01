use std::collections::HashMap;

use askama::Template;
use axum::{extract::State, response::Html, routing::get, Router};

use crate::error::AppError;
use crate::filters; // Custom Askama filters used by templates below.
use crate::services::{
    asset_store, category_store, company_store, pitch_store, sessions, story_store,
};
use crate::startup::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(index))
}

/// One row in the home page's "In progress" feed. Three sources fan into
/// the same shape so the template can render them uniformly.
pub struct InProgressRow {
    /// Display kind: "session", "story", or "pitch".
    pub kind: &'static str,
    /// Lucide icon name used in the kind badge.
    pub icon: &'static str,
    /// Primary line — company name, competency name, or pitch question.
    pub title: String,
    /// Resume target (`/sessions/{id}`, `/stories/{id}`, `/pitches/{slug}`).
    pub href: String,
    /// Sort key — most-recently-touched first.
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Template)]
#[template(path = "home.html")]
struct HomeTemplate {
    company_count: u64,
    primary_asset_present: bool,
    /// Pre-computed badge copy for the Setup cards. Pre-computing here keeps
    /// the template logic-free since Askama 0.12 doesn't support
    /// `if`-expressions inside `{% let %}`.
    openrouter_badge_text: &'static str,
    openrouter_badge_variant: &'static str,
    asset_badge_text: &'static str,
    asset_badge_variant: &'static str,
    active_count: u64,
    in_progress: Vec<InProgressRow>,
}

async fn index(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let company_count = company_store::count(&state.db).await?;
    // Critique flow requires a primary *book*. A resume alone doesn't
    // unblock it; scope the presence check to AssetKind::Book.
    let primary_asset_present =
        asset_store::find_primary_by_kind(&state.pool, crate::models::AssetKind::Book)
            .await?
            .is_some();

    let active_count = sessions::count_active(&state.db).await?;
    let in_progress = load_in_progress(&state).await?;

    let openrouter_configured = state.openrouter.configured();
    crate::error::render_html(HomeTemplate {
        company_count,
        primary_asset_present,
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
        in_progress,
    })
}

/// Fan-in across the three in-progress sources, merged into a single
/// most-recently-touched-first list for the home feed.
async fn load_in_progress(state: &AppState) -> Result<Vec<InProgressRow>, AppError> {
    let (active_sessions, in_progress_stories, in_progress_pitches) = futures::try_join!(
        sessions::list_active(&state.db),
        story_store::list_in_progress(&state.db),
        pitch_store::list_in_progress(&state.db),
    )?;

    let company_by_id = if active_sessions.is_empty() {
        HashMap::new()
    } else {
        let ids: Vec<&str> = active_sessions
            .iter()
            .map(|s| s.company_id.as_str())
            .collect();
        company_store::names_by_ids(&state.db, &ids).await?
    };

    let competency_by_id: HashMap<String, String> = if in_progress_stories.is_empty() {
        HashMap::new()
    } else {
        category_store::list_all(&state.pool)
            .await?
            .into_iter()
            .map(|c| (c.id, c.name))
            .collect()
    };

    let mut rows: Vec<InProgressRow> = Vec::new();
    for s in active_sessions {
        let title = company_by_id
            .get(&s.company_id)
            .cloned()
            .unwrap_or_else(|| "(deleted company)".into());
        rows.push(InProgressRow {
            kind: "session",
            icon: "target",
            title,
            href: format!("/sessions/{}", s.id),
            updated_at: s.started_at,
        });
    }
    for st in in_progress_stories {
        let title = competency_by_id
            .get(&st.competency_id)
            .cloned()
            .unwrap_or_else(|| "(deleted competency)".into());
        rows.push(InProgressRow {
            kind: "story",
            icon: "book-marked",
            title,
            href: format!("/stories/{}", st.id),
            updated_at: st.updated_at,
        });
    }
    for p in in_progress_pitches {
        rows.push(InProgressRow {
            kind: "pitch",
            icon: "mic",
            title: p.question_text,
            href: format!("/pitches/{}", p.slug),
            updated_at: p.updated_at,
        });
    }
    rows.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(rows)
}

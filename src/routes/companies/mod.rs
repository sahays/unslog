//! `/companies/*` — target-company resource: research packet, question bank,
//! and per-company sessions.
//!
//! Split mirrors `routes/sessions/` and `routes/stories/`:
//! * `landing` — list, new form, create.
//! * `show` — company detail page (questions + sessions + research panel).
//! * `questions` — bulk-add, delete, and edit questions on a company.
//! * `actions` — refresh research packet, cascade-delete company.
//!
//! Helpers and shared types used by more than one submodule sit here.

use axum::routing::{get, post};
use axum::Router;

use crate::error::AppError;
use crate::models::{Company, Session, Summary};
use crate::services::company_store;
use crate::startup::AppState;

mod actions;
mod landing;
mod questions;
mod show;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/companies", get(landing::list).post(landing::create))
        .route("/companies/new", get(landing::new_form))
        .route("/companies/:id", get(show::show))
        .route(
            "/companies/:id/refresh-packet",
            post(actions::refresh_packet),
        )
        .route("/companies/:id/delete", post(actions::delete))
        .route("/companies/:id/questions", post(questions::add_questions))
        .route(
            "/companies/:id/questions/:qid/delete",
            post(questions::delete_question),
        )
        .route(
            "/companies/:id/questions/:qid/edit",
            post(questions::edit_question),
        )
}

// ── Shared types ─────────────────────────────────────────────────────────

/// Per-session row rendered on the company show page.
pub struct SessionRow {
    pub session: Session,
    pub eval_count: u64,
    pub summary: Option<Summary>,
}

// ── Helpers shared by landing + show + actions ───────────────────────────

fn role_options() -> Vec<(&'static str, &'static str)> {
    crate::models::Role::ALL
        .iter()
        .map(|r| (r.as_str(), r.display_name()))
        .collect()
}

async fn load_company(state: &AppState, owner_id: &str, id: &str) -> Result<Company, AppError> {
    company_store::find_or_404(&state.pool, owner_id, id).await
}

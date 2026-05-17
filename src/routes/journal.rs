//! `/journal` — personal journal. Plain CRUD with archive (soft-delete).
//! No AI on this side; no integration with stories.

use askama::Template;
use axum::extract::{Form, Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;

use crate::error::AppError;
use crate::models::JournalEntry;
use crate::services::journal_store;
use crate::startup::AppState;

/// Server-side input limits. The form mirrors these via `maxlength`, but the
/// server is the source of truth — never trust client validation.
const MAX_TITLE_LEN: usize = 200;
const MAX_BODY_LEN: usize = 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/journal", get(list).post(create))
        .route("/journal/new", get(new_form))
        .route("/journal/:id", get(show))
        .route("/journal/:id/edit", get(edit_form).post(update))
        .route("/journal/:id/archive", post(archive))
}

// ── Templates ────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "journal/list.html")]
struct ListTemplate {
    entries: Vec<JournalEntry>,
}

#[derive(Template)]
#[template(path = "journal/new.html")]
struct NewTemplate {
    max_title: usize,
    max_body: usize,
}

#[derive(Template)]
#[template(path = "journal/show.html")]
struct ShowTemplate {
    entry: JournalEntry,
}

#[derive(Template)]
#[template(path = "journal/edit.html")]
struct EditTemplate {
    entry: JournalEntry,
    max_title: usize,
    max_body: usize,
}

// ── Handlers ─────────────────────────────────────────────────────────────

async fn list(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let entries = journal_store::list_active(&state.db).await?;
    render(ListTemplate { entries })
}

async fn new_form() -> Result<Html<String>, AppError> {
    render(NewTemplate {
        max_title: MAX_TITLE_LEN,
        max_body: MAX_BODY_LEN,
    })
}

async fn show(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Html<String>, AppError> {
    let entry = require_entry(&state, &id).await?;
    render(ShowTemplate { entry })
}

async fn edit_form(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Html<String>, AppError> {
    let entry = require_entry(&state, &id).await?;
    render(EditTemplate {
        entry,
        max_title: MAX_TITLE_LEN,
        max_body: MAX_BODY_LEN,
    })
}

#[derive(Deserialize)]
pub struct EntryForm {
    pub title: String,
    pub body: String,
}

async fn create(
    State(state): State<AppState>,
    Form(form): Form<EntryForm>,
) -> Result<Response, AppError> {
    let (title, body) = validate(&form)?;
    let entry = journal_store::create(&state.db, title, body).await?;
    tracing::info!(
        event = "journal.entry.created",
        entry_id = %entry.id,
        "journal entry created"
    );
    Ok(Redirect::to("/journal").into_response())
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Form(form): Form<EntryForm>,
) -> Result<Response, AppError> {
    let (title, body) = validate(&form)?;
    let entry = journal_store::update(&state.db, &id, title, body).await?;
    tracing::info!(
        event = "journal.entry.updated",
        entry_id = %entry.id,
        "journal entry updated"
    );
    Ok(Redirect::to(&format!("/journal/{}", entry.id)).into_response())
}

async fn archive(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    journal_store::archive(&state.db, &id).await?;
    tracing::info!(event = "journal.entry.archived", entry_id = %id, "journal entry archived");
    Ok(Redirect::to("/journal").into_response())
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn render<T: Template>(tmpl: T) -> Result<Html<String>, AppError> {
    Ok(Html(
        tmpl.render()
            .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?,
    ))
}

async fn require_entry(state: &AppState, id: &str) -> Result<JournalEntry, AppError> {
    journal_store::get(&state.db, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("journal entry {id}")))
}

fn validate(form: &EntryForm) -> Result<(String, String), AppError> {
    let title = sanitize(&form.title, MAX_TITLE_LEN, "title")?;
    let body = sanitize(&form.body, MAX_BODY_LEN, "body")?;
    Ok((title, body))
}

/// Server-side input check for free-form user text. Defense-in-depth, not the
/// only XSS guard — Askama auto-escapes on output.
///
/// Rules: trim outer whitespace; reject empty / oversized / null bytes /
/// non-whitespace control chars. The control-char check blocks terminal-style
/// injection if anything later logs the raw text.
fn sanitize(input: &str, max_chars: usize, field: &str) -> Result<String, AppError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(format!("{field} is required")));
    }
    if trimmed.chars().count() > max_chars {
        return Err(AppError::BadRequest(format!(
            "{field} must be {max_chars} characters or fewer"
        )));
    }
    if trimmed
        .chars()
        .any(|c| c == '\0' || (c.is_control() && !matches!(c, '\n' | '\t' | '\r')))
    {
        return Err(AppError::BadRequest(format!(
            "{field} contains an invalid character"
        )));
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;

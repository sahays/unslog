//! `GET/POST /agents/:name` — edit form + save (append-only).
//! `GET /agents/:name/new` — same form, blank textarea, nudging the user
//!     to paste a fresh body instead of editing the current one.

use askama::Template;
use axum::extract::{Form, Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;

use super::{active_version_number, describe, MAX_PROMPT_BODY};
use crate::error::AppError;
use crate::models::is_valid_prompt_name;
use crate::services::prompt_store as store;
use crate::services::text_validation;
use crate::startup::AppState;

#[derive(Template)]
#[template(path = "prompts/edit.html")]
struct EditTemplate {
    name: String,
    description: &'static str,
    body: String,
    active_n: u32,
    total_n: u32,
    updated_at: String,
    /// `true` when the textarea starts blank because the user clicked "New
    /// version" — the page copy nudges them toward pasting a fresh body
    /// instead of editing the current one.
    is_blank: bool,
}

pub(super) async fn edit(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Html<String>, AppError> {
    render_edit(&state, &name, false).await
}

/// Same edit form as `edit`, but the textarea starts empty. The POST target
/// is identical (`save`), so submitting still appends a new active version.
pub(super) async fn new_version(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Html<String>, AppError> {
    render_edit(&state, &name, true).await
}

async fn render_edit(
    state: &AppState,
    name: &str,
    is_blank: bool,
) -> Result<Html<String>, AppError> {
    if !is_valid_prompt_name(name) {
        return Err(AppError::NotFound(format!("prompt {name}")));
    }
    let prompt = store::get_prompt(&state.db, name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("prompt {name}")))?;
    let versions = store::list_versions(&state.db, name).await?;
    let total_n = versions.len() as u32;
    let active_n = active_version_number(&versions, &prompt.current_version_id);
    let body = if is_blank {
        String::new()
    } else {
        store::get_current_body(&state.db, name).await?
    };
    crate::error::render_html(EditTemplate {
        name: name.to_string(),
        description: describe(name),
        body,
        active_n,
        total_n,
        updated_at: prompt.updated_at.format("%Y-%m-%d %H:%M UTC").to_string(),
        is_blank,
    })
}

#[derive(Deserialize)]
pub struct SaveForm {
    pub body: String,
}

pub(super) async fn save(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Form(form): Form<SaveForm>,
) -> Result<Response, AppError> {
    if !is_valid_prompt_name(&name) {
        return Err(AppError::NotFound(format!("prompt {name}")));
    }
    let body = text_validation::sanitize_long(&form.body, MAX_PROMPT_BODY, "prompt body")?;
    let body_chars = body.chars().count();
    let version = store::save_version(&state.db, &name, body, None).await?;
    state.prompt_cache.invalidate(&name).await;
    tracing::info!(
        event = "prompt.save",
        prompt = %name,
        version_id = %version.id,
        body_chars,
        "prompt new version saved",
    );
    Ok(Redirect::to(&format!("/agents/{name}")).into_response())
}

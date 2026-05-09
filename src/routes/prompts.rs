use askama::Template;
use axum::extract::{Form, Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;

use crate::error::AppError;
use crate::models::{is_valid_prompt_name, PromptVersion, PROMPT_NAMES};
use crate::services::prompt_store as store;
use crate::startup::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/prompts", get(list))
        .route("/prompts/:name", get(edit).post(save))
        .route("/prompts/:name/history", get(history))
        .route("/prompts/:name/versions/:version_id", get(view_version))
        .route("/prompts/:name/restore/:version_id", post(restore))
}

#[derive(Template)]
#[template(path = "prompts/list.html")]
struct ListTemplate {
    items: Vec<PromptCard>,
}

struct PromptCard {
    name: &'static str,
    description: &'static str,
    body_excerpt: String,
}

fn describe(name: &str) -> &'static str {
    match name {
        "critique" => "Grades each answer against the book's frameworks and the company packet.",
        "research" => "Builds the per-company packet — values, role JD, sample questions, sources.",
        "summary" => {
            "End-of-session debrief with strengths, recurring weaknesses, and blind spots."
        }
        _ => "",
    }
}

async fn list(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let mut items = Vec::new();
    for name in PROMPT_NAMES {
        let body_excerpt = match store::get_prompt(&state.db, name).await? {
            Some(p) => {
                let v = store::get_version(&state.db, &p.current_version_id).await?;
                let body = v.map(|v| v.body).unwrap_or_default();
                excerpt(&body, 280)
            }
            None => String::new(),
        };
        items.push(PromptCard {
            name,
            description: describe(name),
            body_excerpt,
        });
    }
    let body = ListTemplate { items }
        .render()
        .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(body))
}

#[derive(Template)]
#[template(path = "prompts/edit.html")]
struct EditTemplate {
    name: String,
    description: &'static str,
    body: String,
    current_version_id: String,
    updated_at: String,
}

async fn edit(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Html<String>, AppError> {
    if !is_valid_prompt_name(&name) {
        return Err(AppError::NotFound(format!("prompt {name}")));
    }
    let prompt = store::get_prompt(&state.db, &name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("prompt {name}")))?;
    let body = store::get_current_body(&state.db, &name).await?;
    let body = EditTemplate {
        name: name.clone(),
        description: describe(&name),
        body,
        current_version_id: prompt.current_version_id.clone(),
        updated_at: prompt.updated_at.format("%Y-%m-%d %H:%M UTC").to_string(),
    }
    .render()
    .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(body))
}

#[derive(Deserialize)]
pub struct SaveForm {
    pub body: String,
}

async fn save(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Form(form): Form<SaveForm>,
) -> Result<Response, AppError> {
    if !is_valid_prompt_name(&name) {
        return Err(AppError::NotFound(format!("prompt {name}")));
    }
    if form.body.trim().is_empty() {
        return Err(AppError::BadRequest("prompt body cannot be empty".into()));
    }
    let body_chars = form.body.chars().count();
    let version = store::save_version(&state.db, &name, form.body, None).await?;
    tracing::info!(
        event = "prompt.save",
        prompt = %name,
        version_id = %version.id,
        body_chars,
        "prompt new version saved",
    );
    Ok(Redirect::to(&format!("/prompts/{name}")).into_response())
}

#[derive(Template)]
#[template(path = "prompts/history.html")]
struct HistoryTemplate {
    name: String,
    description: &'static str,
    current_version_id: String,
    versions: Vec<PromptVersion>,
}

async fn history(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Html<String>, AppError> {
    if !is_valid_prompt_name(&name) {
        return Err(AppError::NotFound(format!("prompt {name}")));
    }
    let prompt = store::get_prompt(&state.db, &name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("prompt {name}")))?;
    let versions = store::list_versions(&state.db, &name).await?;
    let body = HistoryTemplate {
        name: name.clone(),
        description: describe(&name),
        current_version_id: prompt.current_version_id,
        versions,
    }
    .render()
    .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(body))
}

#[derive(Template)]
#[template(path = "prompts/version.html")]
struct VersionTemplate {
    name: String,
    version: PromptVersion,
    is_current: bool,
}

async fn view_version(
    State(state): State<AppState>,
    Path((name, version_id)): Path<(String, String)>,
) -> Result<Html<String>, AppError> {
    if !is_valid_prompt_name(&name) {
        return Err(AppError::NotFound(format!("prompt {name}")));
    }
    let version = store::get_version(&state.db, &version_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("version {version_id}")))?;
    if version.prompt_name != name {
        return Err(AppError::BadRequest(
            "version does not belong to this prompt".into(),
        ));
    }
    let prompt = store::get_prompt(&state.db, &name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("prompt {name}")))?;
    let is_current = prompt.current_version_id == version.id;
    let body = VersionTemplate {
        name,
        version,
        is_current,
    }
    .render()
    .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(body))
}

async fn restore(
    State(state): State<AppState>,
    Path((name, version_id)): Path<(String, String)>,
) -> Result<Response, AppError> {
    if !is_valid_prompt_name(&name) {
        return Err(AppError::NotFound(format!("prompt {name}")));
    }
    let target = store::get_version(&state.db, &version_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("version {version_id}")))?;
    if target.prompt_name != name {
        return Err(AppError::BadRequest(
            "version does not belong to this prompt".into(),
        ));
    }
    let restored =
        store::save_version(&state.db, &name, target.body, Some(target.id.clone())).await?;
    tracing::info!(
        event = "prompt.restore",
        prompt = %name,
        new_version_id = %restored.id,
        restored_from = %target.id,
        "prompt restored from older version",
    );
    Ok(Redirect::to(&format!("/prompts/{name}/history")).into_response())
}

fn excerpt(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        trimmed.to_string()
    } else {
        let mut out: String = trimmed.chars().take(max_chars).collect();
        out.push('…');
        out
    }
}

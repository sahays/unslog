//! `GET /agents/:name/versions/:version_id` — read-only view of a single
//! prompt version.

use askama::Template;
use axum::extract::{Path, State};
use axum::response::Html;

use super::active_version_number;
use crate::error::AppError;
use crate::filters;
use crate::models::{is_valid_prompt_name, PromptVersion};
use crate::services::prompt_store as store;
use crate::startup::AppState;

#[derive(Template)]
#[template(path = "prompts/version.html")]
struct VersionTemplate {
    name: String,
    version: PromptVersion,
    is_active: bool,
    n: u32,
    total_n: u32,
}

pub(super) async fn view(
    State(state): State<AppState>,
    Path((name, version_id)): Path<(String, String)>,
) -> Result<Html<String>, AppError> {
    if !is_valid_prompt_name(&name) {
        return Err(AppError::NotFound(format!("prompt {name}")));
    }
    let version = store::get_version(&state.pool, &version_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("version {version_id}")))?;
    if version.prompt_name != name {
        return Err(AppError::BadRequest(
            "version does not belong to this prompt".into(),
        ));
    }
    let prompt = store::get_prompt(&state.pool, &name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("prompt {name}")))?;
    let versions = store::list_versions(&state.pool, &name).await?;
    let total_n = versions.len() as u32;
    let n = active_version_number(&versions, &version.id);
    let is_active = prompt.current_version_id == version.id;
    crate::error::render_html(VersionTemplate {
        name,
        version,
        is_active,
        n,
        total_n,
    })
}

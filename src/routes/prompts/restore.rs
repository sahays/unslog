//! `POST /agents/:name/restore/:version_id` — append a new version whose
//! body is copied from an older one. Restoration is itself a new version
//! (preserving the linear, append-only history) — there is no "rewind to
//! version X" that loses everything in between.

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};

use crate::error::AppError;
use crate::models::is_valid_prompt_name;
use crate::services::prompt_store as store;
use crate::startup::AppState;

pub(super) async fn restore(
    State(state): State<AppState>,
    Path((name, version_id)): Path<(String, String)>,
) -> Result<Response, AppError> {
    if !is_valid_prompt_name(&name) {
        return Err(AppError::NotFound(format!("prompt {name}")));
    }
    let target = store::get_version(&state.pool, &version_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("version {version_id}")))?;
    if target.prompt_name != name {
        return Err(AppError::BadRequest(
            "version does not belong to this prompt".into(),
        ));
    }
    let restored =
        store::save_version(&state.pool, &name, target.body, Some(target.id.clone())).await?;
    state.prompt_cache.invalidate(&name).await;
    tracing::info!(
        event = "prompt.restore",
        prompt = %name,
        new_version_id = %restored.id,
        restored_from = %target.id,
        "prompt restored from older version",
    );
    Ok(Redirect::to(&format!("/agents/{name}/history")).into_response())
}

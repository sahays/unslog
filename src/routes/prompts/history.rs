//! `GET /agents/:name/history` — full version list with chronological
//! numbering and "restored from vN" decoration.

use askama::Template;
use axum::extract::{Path, State};
use axum::response::Html;

use super::describe;
use crate::error::AppError;
use crate::filters;
use crate::models::{is_valid_prompt_name, PromptVersion};
use crate::services::prompt_store as store;
use crate::startup::AppState;

#[derive(Template)]
#[template(path = "prompts/history.html")]
struct HistoryTemplate {
    name: String,
    description: &'static str,
    rows: Vec<VersionRow>,
}

/// Display-friendly view of a `PromptVersion` for the history list. Carries
/// the chronological number (`v_n`) so templates can show "v3" instead of a
/// raw UUID, and a pre-resolved `restored_from_n` so the "restored from v2"
/// label is human-readable too.
pub struct VersionRow {
    pub id: String,
    pub n: u32,
    pub is_active: bool,
    pub restored_from_n: Option<u32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub(super) async fn history(
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
    let rows = build_version_rows(&versions, &prompt.current_version_id);
    crate::error::render_html(HistoryTemplate {
        name: name.clone(),
        description: describe(&name),
        rows,
    })
}

/// Build a chronologically-numbered display list from a newest-first
/// `list_versions` result. `v.n` runs 1 (oldest) to total (newest).
fn build_version_rows(versions: &[PromptVersion], active_id: &str) -> Vec<VersionRow> {
    let total = versions.len();
    // First, an id → n lookup so `restored_from` can resolve to a number.
    let id_to_n: std::collections::HashMap<&str, u32> = versions
        .iter()
        .enumerate()
        .map(|(idx, v)| (v.id.as_str(), (total - idx) as u32))
        .collect();
    versions
        .iter()
        .enumerate()
        .map(|(idx, v)| VersionRow {
            n: (total - idx) as u32,
            is_active: v.id == active_id,
            restored_from_n: v
                .restored_from
                .as_deref()
                .and_then(|rid| id_to_n.get(rid).copied()),
            id: v.id.clone(),
            created_at: v.created_at,
        })
        .collect()
}

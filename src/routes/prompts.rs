use askama::Template;
use axum::extract::{Form, Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;

use crate::error::AppError;
use crate::filters; // Custom Askama filters used by templates below.
use crate::models::{is_valid_prompt_name, PromptVersion, PROMPT_NAMES};
use crate::services::prompt_store as store;
use crate::services::text_validation;

/// Hard cap on a single prompt-body version. The bundled prompts under
/// `prompts/*.md` are all well under 10 KB; 50 000 chars leaves room for
/// experimentation while rejecting accidental binary pastes.
const MAX_PROMPT_BODY: usize = 50_000;
use crate::startup::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/agents", get(list))
        .route("/agents/:name", get(edit).post(save))
        .route("/agents/:name/new", get(new_version))
        .route("/agents/:name/history", get(history))
        .route("/agents/:name/versions/:version_id", get(view_version))
        .route("/agents/:name/restore/:version_id", post(restore))
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
    /// Display number of the active version (1-based, in chronological order).
    /// Zero only on never-seeded prompts that shouldn't appear in PROMPT_NAMES.
    active_n: u32,
    /// Total number of versions ever saved for this prompt.
    total_n: u32,
    /// When the prompt was last updated (a new version was activated).
    /// `None` for never-seeded prompts.
    updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn describe(name: &str) -> &'static str {
    match name {
        "critique" => "Grades each answer against the book's frameworks and the company packet.",
        "research" => "Builds the per-company packet — values, role JD, sample questions, sources.",
        "summary" => {
            "End-of-session debrief with strengths, recurring weaknesses, and blind spots."
        }
        "story_chat" => {
            "Story Builder coach — probes and critiques until a STAR+ story is impactful."
        }
        "story_summarize" => "Compresses a Story-Builder chat into a STAR+ bullet version.",
        "story_refine_open" => {
            "Reopens a locked story with one fresh probe to drive a refined version."
        }
        "story_spoken" => {
            "Turns a locked-in STAR+ version into two verbatim-ready spoken monologues (3–5 min + full)."
        }
        "pitch_chat" => {
            "Pitch coach — probes intro / narrative questions (TMAY, why this role, etc.) until distinctive."
        }
        "pitch_lockin" => {
            "Turns a locked pitch chat into two verbatim-ready spoken monologues (≈90s + ≈3 min)."
        }
        _ => "",
    }
}

async fn list(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let mut items = Vec::new();
    for name in PROMPT_NAMES {
        let prompt = store::get_prompt(&state.db, name).await?;
        let (body_excerpt, active_n, total_n, updated_at) = match prompt {
            Some(p) => {
                let versions = store::list_versions(&state.db, name).await?; // newest-first
                let total_n = versions.len() as u32;
                let active_n = active_version_number(&versions, &p.current_version_id);
                let v = store::get_version(&state.db, &p.current_version_id).await?;
                let body = v.map(|v| v.body).unwrap_or_default();
                (excerpt(&body, 280), active_n, total_n, Some(p.updated_at))
            }
            None => (String::new(), 0, 0, None),
        };
        items.push(PromptCard {
            name,
            description: describe(name),
            body_excerpt,
            active_n,
            total_n,
            updated_at,
        });
    }
    let body = ListTemplate { items }
        .render()
        .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(body))
}

/// Compute the chronological display number (1-based, oldest = 1) of the
/// version with `version_id` in the `list_versions`-ordered (newest-first)
/// `versions` slice. Returns 0 if not found.
fn active_version_number(versions: &[PromptVersion], version_id: &str) -> u32 {
    let total = versions.len();
    versions
        .iter()
        .position(|v| v.id == version_id)
        .map(|idx| (total - idx) as u32)
        .unwrap_or(0)
}

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

async fn edit(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Html<String>, AppError> {
    render_edit(&state, &name, false).await
}

/// Same edit form as `edit`, but the textarea starts empty. The POST target
/// is identical (`save`), so submitting still appends a new active version.
async fn new_version(
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
    let rendered = EditTemplate {
        name: name.to_string(),
        description: describe(name),
        body,
        active_n,
        total_n,
        updated_at: prompt.updated_at.format("%Y-%m-%d %H:%M UTC").to_string(),
        is_blank,
    }
    .render()
    .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(rendered))
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
    let body = text_validation::sanitize_long(&form.body, MAX_PROMPT_BODY, "prompt body")?;
    let body_chars = body.chars().count();
    let version = store::save_version(&state.db, &name, body, None).await?;
    tracing::info!(
        event = "prompt.save",
        prompt = %name,
        version_id = %version.id,
        body_chars,
        "prompt new version saved",
    );
    Ok(Redirect::to(&format!("/agents/{name}")).into_response())
}

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
    let rows = build_version_rows(&versions, &prompt.current_version_id);
    let body = HistoryTemplate {
        name: name.clone(),
        description: describe(&name),
        rows,
    }
    .render()
    .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(body))
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

#[derive(Template)]
#[template(path = "prompts/version.html")]
struct VersionTemplate {
    name: String,
    version: PromptVersion,
    is_active: bool,
    n: u32,
    total_n: u32,
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
    let versions = store::list_versions(&state.db, &name).await?;
    let total_n = versions.len() as u32;
    let n = active_version_number(&versions, &version.id);
    let is_active = prompt.current_version_id == version.id;
    let body = VersionTemplate {
        name,
        version,
        is_active,
        n,
        total_n,
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
    Ok(Redirect::to(&format!("/agents/{name}/history")).into_response())
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

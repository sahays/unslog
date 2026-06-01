//! `/agents` — view/edit/version prompts.
//!
//! The resource is large enough to warrant per-handler files:
//! - `list.rs` — `GET /agents` (index card grid)
//! - `edit.rs` — `GET/POST /agents/:name`, `GET /agents/:name/new`
//! - `history.rs` — `GET /agents/:name/history`
//! - `version.rs` — `GET /agents/:name/versions/:version_id`
//! - `restore.rs` — `POST /agents/:name/restore/:version_id`
//!
//! Shared types (the `describe` lookup, the `excerpt` helper, the body
//! cap constant) live here so handler files don't import each other.

use axum::routing::{get, post};
use axum::Router;

use crate::startup::AppState;

mod edit;
mod history;
mod list;
mod restore;
mod version;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/agents", get(list::list))
        .route("/agents/:name", get(edit::edit).post(edit::save))
        .route("/agents/:name/new", get(edit::new_version))
        .route("/agents/:name/history", get(history::history))
        .route("/agents/:name/versions/:version_id", get(version::view))
        .route("/agents/:name/restore/:version_id", post(restore::restore))
        .layer(axum::middleware::from_fn(
            crate::middleware::require_master_middleware,
        ))
}

/// Hard cap on a single prompt-body version. The bundled prompts under
/// `prompts/*.md` are all well under 10 KB; 50 000 chars leaves room for
/// experimentation while rejecting accidental binary pastes.
pub(crate) const MAX_PROMPT_BODY: usize = 50_000;

/// Human-readable one-liner describing what each prompt does — surfaced on
/// the list cards and the edit/history pages so the user doesn't have to
/// remember which agent is which.
pub(crate) fn describe(name: &str) -> &'static str {
    match name {
        "critique" => "Grades each answer against the book's frameworks and the company packet.",
        "research" => "Builds the per-company packet — values, role JD, sample questions, sources.",
        "summary" => {
            "End-of-session debrief with strengths, recurring weaknesses, and blind spots."
        }
        "story_chat" => {
            "Story Builder coach — probes and critiques until a STAR+ story is impactful."
        }
        "story_chat_collaborative" => {
            "Story Builder coach (collaborative mode) — same probing, plus options on request."
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

/// Truncate `text` to `max_chars` USVs, appending a single ellipsis when
/// cut. Used by the list page card excerpts.
pub(crate) fn excerpt(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        trimmed.to_string()
    } else {
        let mut out: String = trimmed.chars().take(max_chars).collect();
        out.push('…');
        out
    }
}

/// Compute the chronological display number (1-based, oldest = 1) of the
/// version with `version_id` in the `list_versions`-ordered (newest-first)
/// `versions` slice. Returns 0 if not found.
pub(crate) fn active_version_number(
    versions: &[crate::models::PromptVersion],
    version_id: &str,
) -> u32 {
    let total = versions.len();
    versions
        .iter()
        .position(|v| v.id == version_id)
        .map(|idx| (total - idx) as u32)
        .unwrap_or(0)
}

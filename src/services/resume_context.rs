//! Build the `<resume>` system-prompt block for coach turns.
//!
//! Reads the primary-resume text via [`ResumeCache`]. Returns an empty
//! string when no resume is set — coach prompts have always worked without
//! a resume and continue to.
//!
//! The `<resume>…</resume>` tag is the only marker that the
//! `redact::preview` scrubber recognizes. Don't change the tag without
//! updating the scrubber, or resume text could leak through log previews.

use std::sync::Arc;

use sqlx::PgPool;

use crate::error::AppError;
use crate::services::assets::ResumeCache;
use crate::startup::AppState;

/// Convenience over [`block_from`]. Pulls the cache out of [`AppState`] and
/// hands it the live pool. `owner_id` is the logged-in user so each
/// account sees their own resume (resumes are per-user).
pub async fn block(state: &AppState, owner_id: &str) -> Result<String, AppError> {
    block_from(&state.resume_cache, &state.pool, owner_id).await
}

/// Build the `<resume>…</resume>` system-prompt block. Returns an empty
/// string when no primary resume exists; callers should skip emitting the
/// surrounding double-newline in that case.
pub async fn block_from(
    cache: &ResumeCache,
    pool: &PgPool,
    owner_id: &str,
) -> Result<String, AppError> {
    let Some(text) = cache.get(pool, owner_id).await? else {
        return Ok(String::new());
    };
    Ok(wrap(&text))
}

/// Pure formatter — same wrapping logic, no I/O. Exposed so unit tests can
/// exercise the tag shape without spinning up a DB.
pub fn wrap(text: &Arc<String>) -> String {
    format!("<resume>\n{}\n</resume>", text.trim())
}

/// Join non-empty prompt blocks with `\n\n`. Skipping empties keeps the
/// composed system prompt clean when an optional block (e.g. `<resume>`)
/// isn't configured. Used by both story and pitch coach prompts.
pub fn join_blocks(blocks: &[&str]) -> String {
    blocks
        .iter()
        .filter(|b| !b.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
#[path = "resume_context_tests.rs"]
mod tests;

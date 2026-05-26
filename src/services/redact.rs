//! Logging discipline helpers. Centralizes the truncation + redaction
//! patterns so logging sites don't ad-hoc them.
//!
//! Use these any time a `tracing::*!` call wants to include user-typed text
//! or LLM output. Logs land in `data/logs/unslog.log` (daily-rotated, never
//! pruned by the app), so anything that lands there is effectively
//! permanent until the user manually wipes the log dir — treat it
//! accordingly.

/// Truncate to `n` Unicode scalar values, appending `…` when cut. Use this
/// for any user-typed field (names, titles, messages) before logging.
/// Default `n` for free-form fields elsewhere in the codebase is 200.
pub fn preview(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "…"
    }
}

#[cfg(test)]
#[path = "redact_tests.rs"]
mod tests;

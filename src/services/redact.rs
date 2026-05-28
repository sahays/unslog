//! Logging discipline helpers. Centralizes the truncation + redaction
//! patterns so logging sites don't ad-hoc them.
//!
//! Use these any time a `tracing::*!` call wants to include user-typed text
//! or LLM output. Logs land in `data/logs/unslog.log` (daily-rotated, never
//! pruned by the app), so anything that lands there is effectively
//! permanent until the user manually wipes the log dir — treat it
//! accordingly.

const RESUME_OPEN: &str = "<resume>";
const RESUME_CLOSE: &str = "</resume>";

/// Truncate to `n` Unicode scalar values, appending `…` when cut. Use this
/// for any user-typed field (names, titles, messages) before logging.
/// Default `n` for free-form fields elsewhere in the codebase is 200.
///
/// Before truncating, content between `<resume>` and `</resume>` is
/// replaced with `[redacted N chars]` so resume text uploaded to `/assets`
/// can't leak through this preview path (used for upstream error bodies
/// in tracing fields, and as a defense-in-depth measure for any future
/// log site that previews chat transcripts).
pub fn preview(s: &str, n: usize) -> String {
    let scrubbed = scrub_resume_tags(s);
    truncate(&scrubbed, n)
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "…"
    }
}

/// Replace every `<resume>…</resume>` body with `[redacted N chars]`. An
/// unclosed open tag is treated as a leak risk: everything after the open
/// tag is redacted in one shot. Case-sensitive — we control the producer
/// (see `services::resume_context::wrap`).
fn scrub_resume_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while let Some(start_rel) = s[i..].find(RESUME_OPEN) {
        let open_at = i + start_rel;
        out.push_str(&s[i..open_at + RESUME_OPEN.len()]);
        let after_open = open_at + RESUME_OPEN.len();
        if let Some(end_rel) = s[after_open..].find(RESUME_CLOSE) {
            let content_end = after_open + end_rel;
            let inner_len = content_end - after_open;
            out.push_str(&format!("[redacted {inner_len} chars]"));
            out.push_str(RESUME_CLOSE);
            i = content_end + RESUME_CLOSE.len();
        } else {
            out.push_str("[redacted — no closing tag]");
            return out;
        }
    }
    out.push_str(&s[i..]);
    out
}

#[cfg(test)]
#[path = "redact_tests.rs"]
mod tests;

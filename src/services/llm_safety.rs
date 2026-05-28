//! LLM output boundary checks. Runs after a model call returns, before we
//! persist or render the result. Catches the two output failure modes that
//! the eval rubric already grades post-hoc but which we want to *also*
//! reject at write time:
//!
//! * **Refusal** — the model says "I cannot help with that" / "as an AI" /
//!   etc. instead of doing the work. Persisting a refusal as a critique or
//!   story summary corrupts the data; we'd rather surface a clean error.
//! * **Prompt leakage** — the model echoes its own system-prompt scaffold
//!   tags (`<competency>`, `<thinking>`, etc.) back into the output, which
//!   signals it confused instructions for content.
//!
//! Refusal markers are lowercase + substring-matched against the **first
//! window** of the response only — refusals always open with one of these
//! phrases. Two defenses against false positives:
//!
//! 1. **Tight window.** Short enough that mid-response phrasing — e.g. a
//!    coach probe like "what's it like to work as an AI engineer?" or a
//!    critique that quotes the candidate's own answer — can't accidentally
//!    fire a marker just by mentioning "AI" or "I cannot" later on.
//! 2. **Specific phrases.** Every marker is the *whole refusal opening*
//!    (e.g. `"as an ai language model"`, `"i cannot help with that"`) not a
//!    fragment (`"as an ai"`, `"i cannot help"`). Fragments matched too many
//!    legitimate uses (a candidate saying "I cannot help feeling…" in a
//!    story, a coach using "as an AI engineer" as context for the user's
//!    own domain). The full opening only appears in actual model refusals.
//!
//! Leakage markers stay as full-text scans since they're explicit tag
//! artifacts we want to strip wherever they appear.

use crate::error::AppError;

/// Window into the start of the response (in chars) used to detect
/// refusal markers. Refusals always open at position 0; 160 chars is
/// enough headroom for soft openings like "I'm sorry, but unfortunately
/// I am not able to assist with that request because…" while still
/// fitting inside almost every short coach response without triggering
/// on later text.
const REFUSAL_WINDOW_CHARS: usize = 160;

/// Refusal *openings*, not fragments. Each marker is a complete phrase
/// the model says when it's declining the task — never something that
/// appears mid-coaching. Substring match (case-insensitive).
const REFUSAL_MARKERS: &[&str] = &[
    "i cannot help with that",
    "i can't help with that",
    "i'm sorry, but i can't",
    "i'm sorry, i can't",
    "i'm sorry, but i cannot",
    "i'm sorry, i cannot",
    "as an ai language model",
    "as an ai assistant, i",
    "as an ai, i cannot",
    "as an ai, i can't",
    "as an ai, i'm not",
    "as an ai, i am not",
    "as a large language model, i",
    "i am not able to assist",
    "i'm not able to assist",
    "i am unable to assist",
    "i'm unable to assist",
    "i cannot fulfill",
    "i can't fulfill",
];

const LEAKAGE_MARKERS: &[&str] = &[
    "<competency>",
    "</competency>",
    "<thinking>",
    "</thinking>",
    "<user_input>",
    "</user_input>",
    "<chat_transcript>",
    "</chat_transcript>",
    "<book_excerpts>",
    "</book_excerpts>",
    "<company_packet>",
    "</company_packet>",
    "<new_attempt>",
    "</new_attempt>",
];

/// What to do when a marker fires. Two policies because the right action
/// differs by marker class.
#[derive(Debug, Clone, Copy)]
pub enum LeakPolicy {
    /// Strip the offending tag substring(s) and return the cleaned text.
    /// Use for leakage markers — the marker itself is the bug, content
    /// around it is usually still salvageable.
    Strip,
    /// Reject the whole output as an error. Use for refusal markers —
    /// a refusal anywhere in the response means the model didn't do the
    /// task and there's nothing useful to keep.
    Reject,
}

/// Validate an LLM response. Returns the cleaned text on success.
/// `op` is a short label that ends up in the rejection error message and
/// the warning log (e.g. "critique", "summary", "story_summarize").
pub fn check_output(op: &str, raw: &str) -> Result<String, AppError> {
    let lower = raw.to_lowercase();
    // Char-aware truncation: byte-slicing `lower` could panic on a
    // multibyte boundary if the response opens with non-ASCII text.
    let refusal_window: String = lower.chars().take(REFUSAL_WINDOW_CHARS).collect();

    for marker in REFUSAL_MARKERS {
        if refusal_window.contains(marker) {
            tracing::warn!(
                op = %op,
                marker = %marker,
                "llm output refused — rejecting",
            );
            return Err(AppError::Upstream(format!(
                "{op} model refused the request (contained {marker:?}); retry or check the prompt"
            )));
        }
    }

    let mut cleaned = raw.to_string();
    let mut stripped_any = false;
    for marker in LEAKAGE_MARKERS {
        if cleaned.contains(marker) {
            stripped_any = true;
            cleaned = cleaned.replace(marker, "");
        }
    }
    if stripped_any {
        tracing::warn!(
            op = %op,
            "llm output contained prompt-leakage tags — stripped before persist",
        );
    }
    Ok(cleaned)
}

#[cfg(test)]
#[path = "llm_safety_tests.rs"]
mod tests;

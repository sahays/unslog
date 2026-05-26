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
//! Refusal markers are lowercase + substring-matched, but only against
//! the **first window** of the response. A genuine refusal opens with one
//! of these phrases; a critique that quotes "as an AI engineer" inside its
//! analysis is not a refusal and shouldn't be rejected. Leakage markers
//! stay as full-text scans since they're explicit tag artifacts we want
//! to strip wherever they appear.

use crate::error::AppError;

/// Window into the start of the response (in chars) used to detect
/// refusal markers. Tuned so a normal model refusal — which always opens
/// with the apology — is caught, while later quotations of the same
/// phrase inside legitimate analysis are not.
const REFUSAL_WINDOW_CHARS: usize = 200;

const REFUSAL_MARKERS: &[&str] = &[
    "i cannot help",
    "i can't help",
    "i'm sorry, but i can't",
    "i'm sorry, i can't",
    "as an ai",
    "as a large language model",
    "i am not able to",
    "i'm not able to",
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

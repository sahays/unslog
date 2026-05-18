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
//! Marker lists are intentionally lowercase + substring-matched after a
//! lowercase normalize — over-matching is fine for refusal (better to
//! reject one borderline output than to persist a polluted one). Leakage
//! markers are exact tags we know we use in prompts.

use crate::error::AppError;

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

    for marker in REFUSAL_MARKERS {
        if lower.contains(marker) {
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

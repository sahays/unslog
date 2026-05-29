//! Lock-in token shared by Stories and Pitches chat flows.
//!
//! Both coach prompts (`story_chat.md`, `pitch_chat.md`) instruct the model
//! to append the literal sentinel below when the candidate has agreed to
//! lock in. The routes used to each carry their own copy of the constant +
//! the strip helper; this centralizes both so the two flows can't drift
//! out of sync.

/// Sentinel the coach emits at the end of its reply when it judges the
/// candidate has agreed to lock in. Spelled out in `prompts/story_chat.md`
/// and `prompts/pitch_chat.md`.
pub const LOCK_IN_TOKEN: &str = "<<LOCK_IN>>";

/// Split a coach reply into its visible content and the lock-in signal.
/// Returns `(cleaned, lock_in_detected)`:
/// * `cleaned` has the token removed (and surrounding whitespace trimmed),
///   so it's safe to persist as a chat turn body.
/// * `lock_in_detected` is `true` when the token appeared anywhere in the
///   reply — the caller then triggers the generate / lock-in flow.
pub fn strip_lock_in_token(s: &str) -> (String, bool) {
    if !s.contains(LOCK_IN_TOKEN) {
        return (s.trim().to_string(), false);
    }
    let cleaned = s.replace(LOCK_IN_TOKEN, "").trim().to_string();
    (cleaned, true)
}

#[cfg(test)]
#[path = "chat_lockin_tests.rs"]
mod tests;

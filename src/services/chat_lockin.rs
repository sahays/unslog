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
mod tests {
    use super::*;

    #[test]
    fn case_no_token_returns_trimmed_passthrough() {
        let (out, locked) = strip_lock_in_token("  hello coach  ");
        assert_eq!(out, "hello coach");
        assert!(!locked);
    }

    #[test]
    fn case_token_at_end_is_stripped_and_flag_true() {
        let (out, locked) = strip_lock_in_token("Great answer. <<LOCK_IN>>");
        assert_eq!(out, "Great answer.");
        assert!(locked);
    }

    #[test]
    fn case_token_in_middle_is_stripped() {
        let (out, locked) = strip_lock_in_token("Yes <<LOCK_IN>> let's go");
        // `replace` + `trim` collapses the trailing whitespace.
        assert_eq!(out, "Yes  let's go");
        assert!(locked);
    }

    #[test]
    fn case_token_only_yields_empty_content() {
        let (out, locked) = strip_lock_in_token("<<LOCK_IN>>");
        assert_eq!(out, "");
        assert!(locked);
    }
}

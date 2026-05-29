use super::*;

/// Lock-in flow: a reply without the sentinel must not trigger the
/// generate / lock-in branch, and the content stored as the chat turn
/// must be trimmed (no trailing model whitespace landing in the DB).
#[test]
fn strip_returns_trimmed_passthrough_and_no_lock_in_when_token_absent() {
    let (out, locked) = strip_lock_in_token("  hello coach  ");
    assert_eq!(out, "hello coach");
    assert!(!locked);
}

/// Lock-in flow: the canonical case — sentinel at the end of the reply
/// must be stripped before the cleaned text is persisted, and the
/// detected flag must be true so the caller fires the lock-in branch.
#[test]
fn strip_removes_trailing_token_and_signals_lock_in_to_caller() {
    let (out, locked) = strip_lock_in_token("Great answer. <<LOCK_IN>>");
    assert_eq!(out, "Great answer.");
    assert!(locked);
}

/// Lock-in flow: a sentinel emitted mid-reply (model formatting quirk)
/// must still be detected and stripped — otherwise the literal
/// `<<LOCK_IN>>` string would be persisted into the chat history.
#[test]
fn strip_removes_mid_reply_token_so_sentinel_never_persists_to_chat_history() {
    let (out, locked) = strip_lock_in_token("Yes <<LOCK_IN>> let's go");
    assert_eq!(out, "Yes  let's go");
    assert!(locked);
}

/// Lock-in flow: a reply that is ONLY the sentinel yields empty
/// content (no persisted turn body) but still flags the lock-in.
#[test]
fn strip_token_only_reply_yields_empty_content_with_lock_in_flagged() {
    let (out, locked) = strip_lock_in_token("<<LOCK_IN>>");
    assert_eq!(out, "");
    assert!(locked);
}

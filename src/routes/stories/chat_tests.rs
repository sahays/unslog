//! Smoke tests for the wire-up between the chat routes and the shared
//! helpers. The strip / render helpers themselves carry exhaustive unit
//! tests in [`crate::services::chat_lockin`] / [`chat_transcript`]; this
//! file mostly guards the imports.

use crate::services::chat_lockin::strip_lock_in_token;
use crate::services::chat_transcript;

// ── strip_lock_in_token (smoke) ──────────────────────────────────────

#[test]
fn case_token_absent() {
    let (cleaned, locked) = strip_lock_in_token("just a normal reply");
    assert_eq!(cleaned, "just a normal reply");
    assert!(!locked);
}

#[test]
fn case_token_present_at_end() {
    let (cleaned, locked) = strip_lock_in_token("text <<LOCK_IN>>");
    assert_eq!(cleaned, "text");
    assert!(locked);
}

// ── chat_transcript::render (smoke) ──────────────────────────────────

#[test]
fn case_empty_chat() {
    assert_eq!(chat_transcript::render(&[]), "");
}

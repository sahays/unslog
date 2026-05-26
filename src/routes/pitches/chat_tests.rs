//! Smoke tests that the route handlers wire the shared lock-in helper
//! correctly. The strip helper itself lives in
//! [`crate::services::chat_lockin`] and carries its own unit tests; this
//! file just guards against accidentally dropping the dependency.

use crate::services::chat_lockin::strip_lock_in_token;

#[test]
fn strip_lock_in_token_absent() {
    let (cleaned, locked) = strip_lock_in_token("just a normal reply");
    assert_eq!(cleaned, "just a normal reply");
    assert!(!locked);
}

#[test]
fn strip_lock_in_token_present_at_end() {
    let (cleaned, locked) = strip_lock_in_token("Locking it in now.\n\n<<LOCK_IN>>");
    assert_eq!(cleaned, "Locking it in now.");
    assert!(locked);
}

#[test]
fn strip_lock_in_token_only() {
    let (cleaned, locked) = strip_lock_in_token("<<LOCK_IN>>");
    assert_eq!(cleaned, "");
    assert!(locked);
}

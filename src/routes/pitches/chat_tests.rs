use super::*;

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

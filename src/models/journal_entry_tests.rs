use super::*;

#[test]
fn case_new_sets_equal_timestamps_and_nonempty_id_and_active() {
    let e = JournalEntry::new("t".into(), "b".into());
    assert!(!e.id.is_empty());
    assert_eq!(e.title, "t");
    assert_eq!(e.body, "b");
    assert_eq!(e.created_at, e.updated_at);
    assert!(e.archived_at.is_none(), "fresh entry is not archived");
}

#[test]
fn case_new_generates_distinct_ids() {
    let a = JournalEntry::new("a".into(), "x".into());
    let b = JournalEntry::new("b".into(), "y".into());
    assert_ne!(a.id, b.id);
}

#[test]
fn case_excerpt_returns_full_trimmed_body_when_short() {
    let e = JournalEntry::new("t".into(), "  hello world  ".into());
    assert_eq!(e.excerpt(), "hello world");
}

#[test]
fn case_excerpt_truncates_with_ellipsis_when_over_limit() {
    let body = "x".repeat(EXCERPT_CHARS + 100);
    let e = JournalEntry::new("t".into(), body);
    let ex = e.excerpt();
    assert_eq!(ex.chars().count(), EXCERPT_CHARS + 1);
    assert!(ex.ends_with('…'));
}

#[test]
fn case_excerpt_preserves_unicode_chars_at_boundary() {
    // Build a body of exactly EXCERPT_CHARS multi-byte chars; should not
    // truncate, and should not panic on byte-vs-char miscounting.
    let body: String = "é".repeat(EXCERPT_CHARS);
    let e = JournalEntry::new("t".into(), body.clone());
    assert_eq!(e.excerpt(), body);
}

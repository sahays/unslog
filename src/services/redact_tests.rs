use super::*;

#[test]
fn case_preview_short_passthrough() {
    assert_eq!(preview("hello", 80), "hello");
}

#[test]
fn case_preview_truncates_with_ellipsis() {
    let s = "a".repeat(100);
    let out = preview(&s, 50);
    assert_eq!(out.chars().count(), 51);
    assert!(out.ends_with('…'));
}

#[test]
fn case_preview_counts_codepoints_not_bytes() {
    let s: String = "é".repeat(60);
    let out = preview(&s, 40);
    assert_eq!(out.chars().count(), 41);
}

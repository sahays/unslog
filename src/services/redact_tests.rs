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

#[test]
fn case_redact_emails_masks_local_part() {
    let out = redact_emails("contact hello@anthropic.com about it");
    assert_eq!(out, "contact h…@anthropic.com about it");
}

#[test]
fn case_redact_emails_leaves_non_email_text() {
    let out = redact_emails("no emails here at all");
    assert_eq!(out, "no emails here at all");
}

#[test]
fn case_redact_emails_keeps_short_addresses_unmasked_when_no_local() {
    // Edge case: "@local-only" isn't a valid email; passes through verbatim.
    let out = redact_emails("@foo");
    assert_eq!(out, "@foo");
}

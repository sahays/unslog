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

// ── Resume-tag scrubbing ─────────────────────────────────────────────────

#[test]
fn case_preview_redacts_resume_block_inner_content() {
    let inner = "my secret content here"; // 22 chars
    let s = format!("before <resume>{inner}</resume> after");
    let out = preview(&s, 1000);
    assert!(!out.contains(inner), "raw resume body must not survive");
    assert!(out.contains("[redacted 22 chars]"));
    assert!(out.contains("<resume>"));
    assert!(out.contains("</resume>"));
    // Surrounding text is preserved verbatim.
    assert!(out.contains("before "));
    assert!(out.contains(" after"));
}

#[test]
fn case_preview_redacts_multiple_resume_blocks() {
    let s = "x <resume>aaa</resume> mid <resume>bbbb</resume> y";
    let out = preview(s, 1000);
    assert!(!out.contains("aaa"));
    assert!(!out.contains("bbbb"));
    assert!(out.contains("[redacted 3 chars]"));
    assert!(out.contains("[redacted 4 chars]"));
    assert!(out.contains(" mid "));
}

#[test]
fn case_preview_unclosed_resume_tag_redacts_remainder() {
    // No closing tag — safe default is to drop the entire tail rather than
    // let user content slip through.
    let s = "prefix <resume>leaked-content-goes-here forever";
    let out = preview(s, 1000);
    assert!(!out.contains("leaked-content-goes-here"));
    assert!(out.contains("[redacted — no closing tag]"));
    assert!(out.starts_with("prefix <resume>"));
}

#[test]
fn case_preview_no_resume_tag_passes_through_unchanged_then_truncates() {
    let s = "no tags here, just text";
    assert_eq!(preview(s, 1000), s);
    // Truncation still applies on plain text.
    let long = "x".repeat(120);
    let out = preview(&long, 50);
    assert_eq!(out.chars().count(), 51);
    assert!(out.ends_with('…'));
}

#[test]
fn case_preview_scrubs_resume_then_applies_truncation() {
    // Scrubbing happens first; if the surrounding text is long enough,
    // the ellipsis still appears at the configured codepoint cutoff.
    let s = format!(
        "{}<resume>secret</resume>{}",
        "a".repeat(40),
        "b".repeat(40)
    );
    let out = preview(&s, 30);
    assert!(!out.contains("secret"));
    assert!(out.ends_with('…'));
}

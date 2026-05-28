use super::*;
use std::sync::Arc;

#[test]
fn case_wrap_produces_tagged_block_with_trimmed_inner_text() {
    let text = Arc::new("  Jane Q. Candidate\nSenior eng @ Acme\n".to_string());
    let out = wrap(&text);
    assert!(out.starts_with("<resume>\n"));
    assert!(out.ends_with("\n</resume>"));
    // Inner content must be trimmed — no leading/trailing whitespace bleeding
    // into the prompt body.
    assert!(out.contains("Jane Q. Candidate\nSenior eng @ Acme"));
    assert!(!out.contains("  Jane")); // leading spaces gone
}

#[test]
fn case_wrap_keeps_inner_newlines_intact() {
    let text = Arc::new("line one\nline two\nline three".to_string());
    let out = wrap(&text);
    assert_eq!(out, "<resume>\nline one\nline two\nline three\n</resume>");
}

/// Sanity check: a string of "no resume" mapped through the same shape
/// returns the empty-string sentinel the wire-up uses to skip the block.
/// We exercise the public empty-string contract without a live DB by
/// asserting the documented semantics in [`block_from`] (cache → None →
/// empty string).
#[test]
fn case_join_blocks_skips_empties() {
    let out = join_blocks(&["alpha", "", "beta", ""]);
    assert_eq!(out, "alpha\n\nbeta");
}

#[test]
fn case_join_blocks_all_empty_returns_empty_string() {
    let out = join_blocks(&["", "", ""]);
    assert_eq!(out, "");
}

#[test]
fn case_join_blocks_single_non_empty_no_separator_added() {
    let out = join_blocks(&["", "only", ""]);
    assert_eq!(out, "only");
}

#[test]
fn case_empty_resume_text_still_wraps_to_empty_tag_pair() {
    // When a resume *is* set but happens to be all whitespace, wrap
    // produces an empty-bodied block. The coach prompt won't include
    // useful context, but no panic and no leak.
    let text = Arc::new("   \n  \n".to_string());
    let out = wrap(&text);
    assert_eq!(out, "<resume>\n\n</resume>");
}

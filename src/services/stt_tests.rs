use super::*;

#[test]
fn case_plain_text() {
    assert_eq!(clean_transcript("hello world"), "hello world");
}

#[test]
fn case_triple_backticks_wrapping() {
    assert_eq!(clean_transcript("```hello```"), "hello");
}

#[test]
fn case_double_quote_wrapping() {
    assert_eq!(clean_transcript("\"hello\""), "hello");
}

#[test]
fn case_fence_then_quote() {
    // Order: trim → strip ``` prefix → strip ``` suffix → trim → strip "
    // prefix → strip " suffix → trim. So both layers come off.
    assert_eq!(clean_transcript("```\"hello\"```"), "hello");
}

#[test]
fn case_only_leading_fence() {
    // strip_prefix succeeds, strip_suffix doesn't match — that's fine.
    assert_eq!(clean_transcript("```hello"), "hello");
}

#[test]
fn case_whitespace_only() {
    assert_eq!(clean_transcript("   "), "");
}

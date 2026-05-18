use super::*;

#[test]
fn case_check_passes_clean_output() {
    let out = check_output("critique", "Great story with concrete details.").expect("ok");
    assert_eq!(out, "Great story with concrete details.");
}

#[test]
fn case_check_rejects_refusal_marker() {
    let err = check_output("critique", "I cannot help with that request.")
        .expect_err("expected upstream error");
    match err {
        AppError::Upstream(msg) => assert!(msg.contains("refused"), "got: {msg}"),
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn case_check_rejects_case_insensitive_refusal() {
    let err = check_output("summary", "As an AI, I should clarify…")
        .expect_err("expected upstream error");
    assert!(matches!(err, AppError::Upstream(_)));
}

#[test]
fn case_check_strips_leakage_tags() {
    let raw = "The candidate said <competency>foo</competency> a lot.";
    let out = check_output("story_summarize", raw).expect("ok");
    assert_eq!(out, "The candidate said foo a lot.");
}

#[test]
fn case_check_strips_multiple_leakage_tags() {
    let raw = "<thinking>internal</thinking> Real summary content.";
    let out = check_output("summary", raw).expect("ok");
    assert_eq!(out, "internal Real summary content.");
}

#[test]
fn case_check_preserves_non_marker_html_like() {
    // `<em>` isn't a marker — it's allowed through. Ammonia handles real
    // HTML sanitization at render time.
    let raw = "Use <em>concrete</em> numbers.";
    let out = check_output("critique", raw).expect("ok");
    assert_eq!(out, raw);
}

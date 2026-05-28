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
    let err = check_output("summary", "As an AI Language Model, I cannot help…")
        .expect_err("expected upstream error");
    assert!(matches!(err, AppError::Upstream(_)));
}

// ── False-positive guards (regressions from the over-broad markers) ──

#[test]
fn case_pass_coach_probe_quoting_ai_engineer() {
    // Before the marker tightening, "as an ai" matched mid-sentence and
    // killed legitimate coach probes like this one.
    let raw =
        "Take me into a moment where, as an AI engineer, you had to push back on a product call.";
    let out = check_output("pitch_chat", raw).expect("should pass");
    assert_eq!(out, raw);
}

#[test]
fn case_pass_candidate_quote_with_cannot_help_phrase() {
    // The candidate uses "I cannot help" idiomatically; the marker requires
    // "i cannot help with that" so this doesn't trip.
    let raw =
        "You said \"I cannot help feeling we should have rolled back sooner.\" What did you do?";
    let out = check_output("story_chat", raw).expect("should pass");
    assert_eq!(out, raw);
}

#[test]
fn case_pass_short_response_with_ai_mention_mid_sentence() {
    // Reproduces the production false-positive: short response (< previous
    // 200-char window), uses "as an AI" mid-sentence, was rejected.
    let raw = "What's the moment, as an AI Solutions Architect, you'd open with?";
    let out = check_output("pitch_chat", raw).expect("should pass");
    assert_eq!(out, raw);
}

#[test]
fn case_pass_reflective_ai_mention_no_refusal() {
    // "As an AI, I should clarify…" used to be flagged — now it's not a
    // refusal because the model is not declining the task; it's just
    // self-identifying. Verbose, but not a refusal. Let it through.
    let raw = "As an AI, I should clarify — what year was this?";
    let out = check_output("story_chat", raw).expect("should pass");
    assert_eq!(out, raw);
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

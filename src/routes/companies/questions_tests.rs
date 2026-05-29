use super::*;

/// Trust-boundary: a normal multi-line paste returns trimmed,
/// non-empty lines in order.
#[test]
fn parse_bulk_questions_returns_trimmed_lines_in_order() {
    let raw =
        "  what is your greatest strength?  \nwhy this company?\n\n   \ntell me a time you failed.";
    let out = parse_bulk_questions(raw).expect("ok");
    assert_eq!(
        out,
        vec![
            "what is your greatest strength?".to_string(),
            "why this company?".to_string(),
            "tell me a time you failed.".to_string(),
        ]
    );
}

/// Trust-boundary: empty or whitespace-only input is rejected so the
/// downstream LLM categorize call is never invoked with no questions.
#[test]
fn parse_bulk_questions_rejects_empty_or_whitespace_only_input() {
    assert!(matches!(
        parse_bulk_questions("").expect_err("empty"),
        AppError::BadRequest(_)
    ));
    assert!(matches!(
        parse_bulk_questions("   \n\n   \n").expect_err("blank"),
        AppError::BadRequest(_)
    ));
}

/// DoS defense: a paste containing more than [`MAX_BULK_QUESTIONS`]
/// non-empty lines is rejected — otherwise a 5 MB paste fans out to
/// thousands of categorize LLM calls.
#[test]
fn parse_bulk_questions_rejects_over_cap_so_a_huge_paste_cannot_fan_out_to_llm() {
    let raw: String = (0..(MAX_BULK_QUESTIONS + 1))
        .map(|i| format!("q{i}?\n"))
        .collect();
    let err = parse_bulk_questions(&raw).expect_err("expected cap rejection");
    assert!(matches!(err, AppError::BadRequest(msg) if msg.contains("cap is")));
}

/// Trust-boundary: a paste at exactly the cap is accepted (off-by-one
/// guard).
#[test]
fn parse_bulk_questions_accepts_exactly_the_cap_off_by_one_guard() {
    let raw: String = (0..MAX_BULK_QUESTIONS)
        .map(|i| format!("q{i}?\n"))
        .collect();
    let out = parse_bulk_questions(&raw).expect("ok at cap");
    assert_eq!(out.len(), MAX_BULK_QUESTIONS);
}

/// Trust-boundary: a single line that exceeds [`MAX_QUESTION_TEXT`]
/// is rejected (sanitize_short fires) — defends against an essay
/// pasted as a single line, which would also blow categorize tokens.
#[test]
fn parse_bulk_questions_rejects_individual_line_over_question_text_cap() {
    let huge = "a".repeat(MAX_QUESTION_TEXT + 1);
    let err = parse_bulk_questions(&huge).expect_err("oversize line");
    assert!(matches!(err, AppError::BadRequest(_)));
}

/// Trust-boundary: control characters in a question line are
/// rejected via the underlying short-sanitizer (the question text
/// flows into prompts and renders, so a null byte / ESC must not
/// land in storage).
#[test]
fn parse_bulk_questions_rejects_control_chars_via_short_sanitizer() {
    let err = parse_bulk_questions("hi\0there?").expect_err("null byte");
    assert!(matches!(err, AppError::BadRequest(_)));
    let err = parse_bulk_questions("hi\x1bescape?").expect_err("esc");
    assert!(matches!(err, AppError::BadRequest(_)));
}

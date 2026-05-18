use super::*;

#[test]
fn case_short_accepts_within_limits() {
    let out = sanitize_short("  Anthropic  ", 200, "name").expect("ok");
    assert_eq!(out, "Anthropic");
}

#[test]
fn case_short_rejects_newline() {
    let err = sanitize_short("two\nlines", 200, "name").expect_err("expected bad request");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_short_rejects_tab() {
    let err = sanitize_short("with\ttab", 200, "name").expect_err("expected bad request");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_short_rejects_null_byte() {
    let err = sanitize_short("hi\0bye", 200, "name").expect_err("expected bad request");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_short_rejects_empty_after_trim() {
    let err = sanitize_short("   ", 200, "name").expect_err("expected bad request");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_short_rejects_oversize() {
    let s = "a".repeat(201);
    let err = sanitize_short(&s, 200, "name").expect_err("expected bad request");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_long_accepts_multiline_with_tabs() {
    let out = sanitize_long("line one\nline two\n\tindented", 1024, "body").expect("ok");
    assert_eq!(out, "line one\nline two\n\tindented");
}

#[test]
fn case_long_normalizes_crlf() {
    // CRLF → LF; trimmed length matches what the textarea maxlength sees.
    let out = sanitize_long("line\r\ntwo", 1024, "body").expect("ok");
    assert_eq!(out, "line\ntwo");
}

#[test]
fn case_long_rejects_escape_control() {
    let err = sanitize_long("hi\x1bthere", 1024, "body").expect_err("expected bad request");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_long_rejects_null_byte() {
    let err = sanitize_long("body\0sneak", 1024, "body").expect_err("expected bad request");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_long_unicode_char_count_not_byte_count() {
    // 1000 multi-byte chars under a 1024 char limit must pass even though
    // the byte length is well over the limit.
    let s: String = "é".repeat(1000);
    let out = sanitize_long(&s, 1024, "body").expect("ok");
    assert_eq!(out.chars().count(), 1000);
}

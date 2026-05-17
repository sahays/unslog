use super::*;

fn form(title: &str, body: &str) -> EntryForm {
    EntryForm {
        title: title.into(),
        body: body.into(),
    }
}

#[test]
fn case_validate_ok_trims_both_fields() {
    let (t, b) = validate(&form("  hello  ", "\n  world\t")).expect("ok");
    assert_eq!(t, "hello");
    assert_eq!(b, "world");
}

#[test]
fn case_validate_rejects_empty_title() {
    let err = validate(&form("", "body")).expect_err("expected bad request");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_validate_rejects_whitespace_only_title() {
    let err = validate(&form("   \t\n", "body")).expect_err("expected bad request");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_validate_rejects_empty_body() {
    let err = validate(&form("title", "")).expect_err("expected bad request");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_validate_rejects_whitespace_only_body() {
    let err = validate(&form("title", "  \n  ")).expect_err("expected bad request");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_validate_rejects_oversized_title() {
    let long = "a".repeat(MAX_TITLE_LEN + 1);
    let err = validate(&form(&long, "body")).expect_err("expected bad request");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_validate_accepts_max_size_title() {
    let exact = "a".repeat(MAX_TITLE_LEN);
    let (t, _) = validate(&form(&exact, "body")).expect("ok");
    assert_eq!(t.chars().count(), MAX_TITLE_LEN);
}

#[test]
fn case_validate_rejects_oversized_body() {
    let long = "a".repeat(MAX_BODY_LEN + 1);
    let err = validate(&form("title", &long)).expect_err("expected bad request");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_validate_accepts_max_size_body() {
    let exact = "a".repeat(MAX_BODY_LEN);
    let (_, b) = validate(&form("title", &exact)).expect("ok");
    assert_eq!(b.chars().count(), MAX_BODY_LEN);
}

#[test]
fn case_validate_rejects_null_byte_in_title() {
    let err = validate(&form("hi\0there", "body")).expect_err("expected bad request");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_validate_rejects_null_byte_in_body() {
    let err = validate(&form("title", "body\0sneak")).expect_err("expected bad request");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_validate_rejects_escape_control_char_in_title() {
    // ESC (0x1b) — terminal injection vector if anything ever logs the raw text.
    let err = validate(&form("hi\x1bthere", "body")).expect_err("expected bad request");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_validate_accepts_newlines_and_tabs_in_body() {
    let body = "line one\nline two\n\tindented\rcr";
    let (_, b) = validate(&form("title", body)).expect("ok");
    assert_eq!(b, body);
}

#[test]
fn case_validate_does_not_strip_html_like_input() {
    // We rely on output escaping (Askama auto-escape) for XSS, not input
    // stripping — the user can legitimately write "<my idea>" in prose.
    let (t, b) = validate(&form("<plan>", "body with <script>alert(1)</script>")).expect("ok");
    assert_eq!(t, "<plan>");
    assert!(b.contains("<script>"));
}

#[test]
fn case_validate_oversized_trims_first() {
    // Oversized only matters after trimming — leading/trailing whitespace
    // shouldn't push a borderline-valid title over the limit.
    let padded = format!("   {}   ", "a".repeat(MAX_TITLE_LEN));
    let (t, _) = validate(&form(&padded, "body")).expect("ok");
    assert_eq!(t.chars().count(), MAX_TITLE_LEN);
}

//! Handler-layer integration tests for `validate()`. The lower-level
//! string sanitizers (null bytes, control chars, CRLF, oversize, unicode)
//! are already covered exhaustively in `services::text_validation`; these
//! tests pin only the handler-specific decisions that aren't visible at
//! the sanitizer layer:
//!   - trim both fields together
//!   - trim before checking the length cap
//!   - pass HTML-like input through (XSS defense is Askama auto-escape on output)
//!   - wire the sanitizer in at all (one negative test as a smoke check)

use super::*;

fn form(title: &str, body: &str) -> EntryForm {
    EntryForm {
        title: title.into(),
        body: body.into(),
    }
}

/// Handler integration: both title and body go through trim before storage.
#[test]
fn validate_handler_trims_both_form_fields_before_storage() {
    let (t, b) = validate(&form("  hello  ", "\n  world\t")).expect("ok");
    assert_eq!(t, "hello");
    assert_eq!(b, "world");
}

/// Handler ordering: leading/trailing whitespace must not push a
/// borderline-valid title over the length cap — trim happens before cap-check.
#[test]
fn validate_handler_trims_before_checking_length_cap() {
    let padded = format!("   {}   ", "a".repeat(MAX_TITLE_LEN));
    let (t, _) = validate(&form(&padded, "body")).expect("ok");
    assert_eq!(t.chars().count(), MAX_TITLE_LEN);
}

/// XSS strategy pin: we deliberately do NOT strip HTML-like input at the
/// input boundary — defense is Askama auto-escape at the output sink. If a
/// future change starts stripping `<` and `>` here, legitimate prose like
/// "<my idea>" silently mutates.
#[test]
fn validate_handler_passes_html_like_input_through_relying_on_askama_escape() {
    let (t, b) = validate(&form("<plan>", "body with <script>alert(1)</script>")).expect("ok");
    assert_eq!(t, "<plan>");
    assert!(b.contains("<script>"));
}

/// Wiring smoke check: confirms `validate()` actually calls the
/// sanitizer (not just trims). If a refactor accidentally drops the
/// sanitize call, null bytes would slip through and reach storage / logs.
#[test]
fn validate_handler_invokes_sanitizer_so_null_bytes_rejected() {
    let err = validate(&form("title", "body\0sneak")).expect_err("expected bad request");
    assert!(matches!(err, AppError::BadRequest(_)));
}

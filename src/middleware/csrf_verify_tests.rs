//! Tests for the CSRF verify middleware. These cover the pure decision
//! function (`decide`) plus the small helpers — no router stand-up is
//! needed since `decide` returns an enum that the HTTP layer translates
//! to 200/403/413. Token transport (header vs form body) is exercised
//! against both happy and adversarial inputs.

use super::*;
use crate::services::csrf;
use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, Method};

const KEY: &[u8] = b"0123456789abcdef0123456789abcdef";
const COOKIE_NAME: &str = "__Host-csrf";

// ── Pure-helper tests ────────────────────────────────────────────────

#[test]
fn case_is_state_changing_only_for_body_verbs() {
    assert!(is_state_changing(&Method::POST));
    assert!(is_state_changing(&Method::PUT));
    assert!(is_state_changing(&Method::PATCH));
    assert!(is_state_changing(&Method::DELETE));
    assert!(!is_state_changing(&Method::GET));
    assert!(!is_state_changing(&Method::HEAD));
    assert!(!is_state_changing(&Method::OPTIONS));
}

#[test]
fn case_is_csrf_exempt_only_for_pre_auth_routes() {
    assert!(is_csrf_exempt("/login"));
    assert!(is_csrf_exempt("/health"));
    assert!(is_csrf_exempt("/static/css/app.css"));
    // /logout has a cookie post-login — must be verified.
    assert!(!is_csrf_exempt("/logout"));
    assert!(!is_csrf_exempt("/sessions/abc/end"));
    assert!(!is_csrf_exempt("/"));
}

#[test]
fn case_extract_form_field_returns_csrf_token_value() {
    let body = b"foo=bar&csrf_token=abc.def.ghi&baz=qux";
    assert_eq!(
        extract_form_field(body, "csrf_token"),
        Some("abc.def.ghi".to_string())
    );
}

#[test]
fn case_extract_form_field_returns_none_when_absent() {
    let body = b"foo=bar&baz=qux";
    assert_eq!(extract_form_field(body, "csrf_token"), None);
}

// ── Decision tests ────────────────────────────────────────────────────

fn form_headers(cookie: Option<&str>, x_csrf: Option<&str>) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded"),
    );
    if let Some(c) = cookie {
        h.insert(header::COOKIE, HeaderValue::from_str(c).unwrap());
    }
    if let Some(t) = x_csrf {
        h.insert("x-csrf-token", HeaderValue::from_str(t).unwrap());
    }
    h
}

fn multipart_headers(cookie: Option<&str>, x_csrf: Option<&str>) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("multipart/form-data; boundary=----xyz"),
    );
    if let Some(c) = cookie {
        h.insert(header::COOKIE, HeaderValue::from_str(c).unwrap());
    }
    if let Some(t) = x_csrf {
        h.insert("x-csrf-token", HeaderValue::from_str(t).unwrap());
    }
    h
}

#[tokio::test]
async fn case_decide_rejects_when_cookie_missing() {
    let headers = form_headers(None, None);
    let (decision, _) = decide(&headers, Body::from("csrf_token=anything"), COOKIE_NAME).await;
    assert!(matches!(decision, Decision::Reject("csrf.cookie_missing")));
}

#[tokio::test]
async fn case_decide_rejects_when_no_form_or_header_token() {
    let tok = csrf::mint("u_1", KEY);
    let cookie_hdr = format!("{COOKIE_NAME}={tok}");
    let headers = form_headers(Some(&cookie_hdr), None);
    let (decision, _) = decide(&headers, Body::from("foo=bar"), COOKIE_NAME).await;
    assert!(matches!(decision, Decision::Reject("csrf.token_missing")));
}

#[tokio::test]
async fn case_decide_verifies_with_matching_form_token() {
    let tok = csrf::mint("u_1", KEY);
    let cookie_hdr = format!("{COOKIE_NAME}={tok}");
    let headers = form_headers(Some(&cookie_hdr), None);
    let body = Body::from(format!("csrf_token={tok}&extra=v"));
    let (decision, _) = decide(&headers, body, COOKIE_NAME).await;
    match decision {
        Decision::Verify { form, cookie } => {
            assert_eq!(form, tok);
            assert_eq!(cookie, tok);
            // End-to-end: the verify call must accept these.
            assert!(csrf::verify(&form, &cookie, KEY).is_ok());
        }
        other => panic!("expected Verify, got {other:?}"),
    }
}

#[tokio::test]
async fn case_decide_verifies_with_matching_header_token() {
    let tok = csrf::mint("u_1", KEY);
    let cookie_hdr = format!("{COOKIE_NAME}={tok}");
    let headers = multipart_headers(Some(&cookie_hdr), Some(&tok));
    let (decision, _) = decide(&headers, Body::empty(), COOKIE_NAME).await;
    match decision {
        Decision::Verify { form, cookie } => {
            assert_eq!(form, tok);
            assert_eq!(cookie, tok);
        }
        other => panic!("expected Verify, got {other:?}"),
    }
}

#[tokio::test]
async fn case_decide_mismatched_tokens_surface_as_verify_then_fail() {
    let cookie_tok = csrf::mint("u_1", KEY);
    let form_tok = csrf::mint("u_1", KEY); // different nonce → must fail csrf::verify
    let cookie_hdr = format!("{COOKIE_NAME}={cookie_tok}");
    let headers = form_headers(Some(&cookie_hdr), None);
    let body = Body::from(format!("csrf_token={form_tok}"));
    let (decision, _) = decide(&headers, body, COOKIE_NAME).await;
    match decision {
        Decision::Verify { form, cookie } => {
            assert_ne!(form, cookie);
            // Middleware would translate this into a 403; verify the
            // underlying check does reject the mismatched pair.
            assert!(csrf::verify(&form, &cookie, KEY).is_err());
        }
        other => panic!("expected Verify, got {other:?}"),
    }
}

#[tokio::test]
async fn case_decide_oversize_form_body_returns_oversize() {
    let tok = csrf::mint("u_1", KEY);
    let cookie_hdr = format!("{COOKIE_NAME}={tok}");
    let headers = form_headers(Some(&cookie_hdr), None);
    // Form-encoded body > 1MB cap. Must not be silently forwarded.
    let big = "x".repeat(2 * 1024 * 1024);
    let body = Body::from(format!("csrf_token={tok}&padding={big}"));
    let (decision, _) = decide(&headers, body, COOKIE_NAME).await;
    assert!(matches!(decision, Decision::Oversize));
}

#[tokio::test]
async fn case_decide_multipart_without_header_is_token_missing() {
    // Multipart audio must use the header path — without it we never buffer
    // the multipart body, so the decision is "token missing", not "oversize".
    let tok = csrf::mint("u_1", KEY);
    let cookie_hdr = format!("{COOKIE_NAME}={tok}");
    let headers = multipart_headers(Some(&cookie_hdr), None);
    let (decision, _) = decide(&headers, Body::from("--xyz--"), COOKIE_NAME).await;
    assert!(matches!(decision, Decision::Reject("csrf.token_missing")));
}

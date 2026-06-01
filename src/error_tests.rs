use super::*;
use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::IntoResponse;

const HX_REDIRECT: &str = "hx-redirect";

#[test]
fn case_unauthenticated_status_is_401() {
    assert_eq!(AppError::Unauthenticated.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn case_forbidden_status_is_403() {
    assert_eq!(AppError::Forbidden.status(), StatusCode::FORBIDDEN);
}

#[test]
fn case_rate_limited_status_is_429() {
    let err = AppError::RateLimited {
        used: 100,
        cap: 200,
    };
    assert_eq!(err.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn case_unauthenticated_html_redirects_to_login() {
    // No IS_HTMX scope set ⇒ full-page path ⇒ 303 to /login.
    let resp = AppError::Unauthenticated.into_response();
    let status = resp.status();
    let location = resp
        .headers()
        .get(axum::http::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(status.is_redirection(), "want 3xx, got {status}");
    assert_eq!(location, "/login");
}

#[tokio::test]
async fn case_unauthenticated_htmx_uses_hx_redirect_header() {
    let resp = crate::middleware::request_context::IS_HTMX
        .scope(true, async { AppError::Unauthenticated.into_response() })
        .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let hx = resp
        .headers()
        .get(HX_REDIRECT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(hx, "/login");
}

#[tokio::test]
async fn case_rate_limited_body_html_escapes_payload() {
    // Render through the HTMX path so the body is the small fragment we
    // can scan. The fragment goes through `html_escape`; verify a script
    // payload is escaped, not emitted raw.
    let body = htmx_error(
        StatusCode::TOO_MANY_REQUESTS,
        "Rate limit reached",
        "<script>alert(1)</script>",
    )
    .into_response();
    let bytes = to_bytes(body.into_body(), 8 * 1024).await.unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(!text.contains("<script>"), "raw script tag in body: {text}");
    assert!(
        text.contains("&lt;script&gt;"),
        "expected escaped tag: {text}"
    );
}

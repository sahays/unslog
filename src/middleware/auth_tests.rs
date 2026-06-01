//! Logic tests for the middleware's pure helpers. End-to-end coverage
//! (cookie + extension wiring) lives in the route tests for `/login` and
//! `/logout`; here we focus on the bits that don't need an axum `State`.

use super::*;
use axum::http::StatusCode;

#[test]
fn case_is_allowlisted_accepts_login_and_static() {
    assert!(is_allowlisted("/login"));
    assert!(is_allowlisted("/logout"));
    assert!(is_allowlisted("/health"));
    assert!(is_allowlisted("/static/css/app.css"));
    assert!(is_allowlisted("/recordings/sess_x/q_y.mp3"));
}

#[test]
fn case_is_allowlisted_rejects_protected_paths() {
    assert!(!is_allowlisted("/"));
    assert!(!is_allowlisted("/companies"));
    assert!(!is_allowlisted("/login/extra"));
    assert!(!is_allowlisted("/sessions/abc"));
}

#[test]
fn case_unauthenticated_redirect_full_page_is_303_to_login() {
    let resp = unauthenticated_redirect(false);
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let loc = resp.headers().get(axum::http::header::LOCATION);
    assert_eq!(loc.and_then(|v| v.to_str().ok()), Some("/login"));
}

#[test]
fn case_unauthenticated_redirect_htmx_uses_hx_redirect() {
    let resp = unauthenticated_redirect(true);
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let hx = resp.headers().get("hx-redirect");
    assert_eq!(hx.and_then(|v| v.to_str().ok()), Some("/login"));
}

#[tokio::test]
async fn case_task_locals_default_to_empty_outside_scope() {
    assert_eq!(current_user_label(), "");
    assert_eq!(current_csrf_token(), "");
}

#[tokio::test]
async fn case_task_locals_visible_inside_scope() {
    CURRENT_USER_LABEL
        .scope("guest 1".to_string(), async {
            CURRENT_CSRF_TOKEN
                .scope("csrf-abc".to_string(), async {
                    assert_eq!(current_user_label(), "guest 1");
                    assert_eq!(current_csrf_token(), "csrf-abc");
                })
                .await
        })
        .await;
}

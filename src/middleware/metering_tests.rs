//! Logic tests for the metering middleware's pure helpers + the
//! UsageWindow task-local. End-to-end (cap-check on a real router) is
//! exercised by the `cap` service unit tests against `MockMeteringDeps`;
//! here we focus on what doesn't need an axum `State`.

use super::*;
use crate::models::UsageWindow;

#[test]
fn case_is_metering_exempt_allows_static_and_recordings() {
    assert!(is_metering_exempt("/login"));
    assert!(is_metering_exempt("/logout"));
    assert!(is_metering_exempt("/health"));
    assert!(is_metering_exempt("/static/css/app.css"));
    assert!(is_metering_exempt("/recordings/sess/file.mp3"));
}

#[test]
fn case_is_metering_exempt_blocks_protected_paths() {
    assert!(!is_metering_exempt("/"));
    assert!(!is_metering_exempt("/companies"));
    assert!(!is_metering_exempt("/sessions/abc"));
}

#[tokio::test]
async fn case_current_user_usage_defaults_outside_scope() {
    let w = current_user_usage();
    assert_eq!(w.used, 0);
    assert_eq!(w.cap, u64::MAX);
}

#[tokio::test]
async fn case_current_user_usage_visible_inside_scope() {
    CURRENT_USER_USAGE
        .scope(UsageWindow { used: 42, cap: 100 }, async {
            let w = current_user_usage();
            assert_eq!(w.used, 42);
            assert_eq!(w.cap, 100);
        })
        .await;
}

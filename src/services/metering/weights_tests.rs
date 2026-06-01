use super::*;
use axum::http::Method;

#[test]
fn case_path_matches_literal_segments() {
    assert!(path_matches("/sessions/:id/end", "/sessions/abc/end"));
    assert!(path_matches("/health", "/health"));
}

#[test]
fn case_path_matches_rejects_wrong_arity() {
    assert!(!path_matches("/sessions/:id", "/sessions/abc/end"));
    assert!(!path_matches("/sessions/:id/end", "/sessions/abc"));
}

#[test]
fn case_path_matches_wildcard_segment_requires_value() {
    assert!(!path_matches("/sessions/:id/end", "/sessions//end"));
}

#[test]
fn case_weight_for_known_billed_route_returns_units() {
    let (kind, units) = weight_for(&Method::POST, "/sessions/sess123/answers");
    assert_eq!(kind, RequestKind::Llm);
    assert_eq!(units, 5);
}

#[test]
fn case_weight_for_unknown_route_returns_free_page_nav() {
    let (kind, units) = weight_for(&Method::GET, "/companies");
    assert_eq!(kind, RequestKind::PageNav);
    assert_eq!(units, 0);
}

#[test]
fn case_weight_for_wrong_method_returns_free() {
    // GET /sessions/:id/end isn't a route at all, but the table only
    // matches POST. GET must read as free.
    let (_, units) = weight_for(&Method::GET, "/sessions/abc/end");
    assert_eq!(units, 0);
}

#[test]
fn case_weight_for_multiple_dynamic_segments() {
    let (kind, units) = weight_for(
        &Method::POST,
        "/sessions/sess1/attempts/ev1/2/regenerate-audio",
    );
    assert_eq!(kind, RequestKind::Tts);
    assert_eq!(units, 1);
}

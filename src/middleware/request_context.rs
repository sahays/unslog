//! Per-request UUID v7 + tracing span + completion log.
//!
//! Adapted from glimmer's pattern. Every HTTP request gets:
//!   • a `request_id` (honors incoming `x-request-id` if present, else mints
//!     a fresh UUID v7),
//!   • a `tracing::info_span!("http_request", request_id, method, path)`
//!     covering the entire downstream handler chain,
//!   • a single `request_completed` info log with `status` + `duration_ms`,
//!   • an `x-request-id` echo header so curl/devtools can correlate.

use axum::body::Body;
use axum::http::{header::HeaderName, HeaderValue, Request, Response};
use axum::middleware::Next;
use std::time::Instant;
use tracing::Instrument;
use uuid::Uuid;

const X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// Carried through the request lifecycle via extensions, so handlers and
/// services that need to log can pull the request_id out.
#[derive(Clone, Debug)]
pub struct RequestContext {
    pub request_id: Uuid,
}

pub async fn request_context_middleware(mut request: Request<Body>, next: Next) -> Response<Body> {
    let request_id = request
        .headers()
        .get(&X_REQUEST_ID)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<Uuid>().ok())
        .unwrap_or_else(Uuid::now_v7);

    let ctx = RequestContext { request_id };
    request.extensions_mut().insert(ctx);

    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let start = Instant::now();

    let span = tracing::info_span!(
        "http_request",
        request_id = %request_id,
        method = %method,
        path = %path,
    );

    let mut response = next.run(request).instrument(span).await;
    let duration_ms = start.elapsed().as_millis() as u64;
    let status = response.status().as_u16();

    // Skip the static-asset and recordings noise — those are routine
    // browser fetches and would drown the log.
    let is_noisy_static = path.starts_with("/static/")
        || path.starts_with("/recordings/")
        || path == "/health";

    if !is_noisy_static {
        tracing::info!(
            request_id = %request_id,
            method = %method,
            path = %path,
            status,
            duration_ms,
            "request_completed"
        );
    } else {
        tracing::debug!(
            request_id = %request_id,
            method = %method,
            path = %path,
            status,
            duration_ms,
            "request_completed"
        );
    }

    if let Ok(hv) = HeaderValue::from_str(&request_id.to_string()) {
        response.headers_mut().insert(X_REQUEST_ID, hv);
    }

    response
}

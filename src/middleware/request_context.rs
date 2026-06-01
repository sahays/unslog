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
const HX_REQUEST: HeaderName = HeaderName::from_static("hx-request");

tokio::task_local! {
    /// Per-request UUID, set inside `request_context_middleware` and read by
    /// the error renderer (which has no access to request extensions).
    pub static REQUEST_ID: Uuid;
    /// Whether the in-flight request carried `hx-request: true`. Lets
    /// `AppError::into_response` pick a fragment vs full-page reply without
    /// threading the request through.
    pub static IS_HTMX: bool;
}

/// Best-effort lookup. Returns the empty string outside a request scope —
/// the error template tolerates that.
pub fn current_request_id() -> String {
    REQUEST_ID.try_with(|id| id.to_string()).unwrap_or_default()
}

/// Best-effort lookup of the in-flight request's `hx-request` header. Returns
/// `false` outside a request scope — safe default since a full-page render is
/// always acceptable.
pub fn current_is_htmx() -> bool {
    IS_HTMX.try_with(|v| *v).unwrap_or(false)
}

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

    let is_htmx = request
        .headers()
        .get(&HX_REQUEST)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

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

    let mut response = REQUEST_ID
        .scope(
            request_id,
            IS_HTMX.scope(is_htmx, next.run(request).instrument(span)),
        )
        .await;
    let duration_ms = start.elapsed().as_millis() as u64;
    let status = response.status().as_u16();

    // Skip the static-asset and recordings noise — those are routine
    // browser fetches and would drown the log.
    let is_noisy_static =
        path.starts_with("/static/") || path.starts_with("/recordings/") || path == "/health";

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

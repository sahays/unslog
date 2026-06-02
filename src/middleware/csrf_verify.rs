//! Double-submit-cookie CSRF verification.
//!
//! Layered AFTER `auth_middleware` so rejections can attribute to a user
//! where possible. Only state-changing verbs are inspected; reads pass
//! through. A tiny allowlist of pre-auth endpoints (`/login`, `/health`,
//! `/static/*`) bypasses verification because no `__Host-csrf` cookie is
//! issued before sign-in.
//!
//! Token transport:
//!   * `X-CSRF-Token` header — preferred (HTMX + multipart audio uploads).
//!   * Form-encoded body field `csrf_token` — classic HTML forms.
//!
//! Body buffering is capped at 1 MB. Bigger state-changing form requests
//! (which would have to be application/x-www-form-urlencoded — multipart
//! takes the header path) are refused with 413 because we can't verify
//! the embedded token without buffering, and silently forwarding an
//! un-verified state-changing request would defeat the control.

use axum::body::{to_bytes, Body};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::cookie::CookieJar;

use crate::error::AppError;
use crate::services::auth::CurrentUser;
use crate::services::csrf;
use crate::startup::AppState;

const CSRF_HEADER: &str = "x-csrf-token";
const CSRF_FORM_FIELD: &str = "csrf_token";
const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";
/// Cap how much body we'll buffer to extract the CSRF token. Larger
/// state-changing form posts are refused; multipart audio takes the
/// header path and is unaffected.
const MAX_BUFFERED_BODY: usize = 1024 * 1024;

/// Outcome of token resolution; `Decision::Verify` carries the inputs to
/// the constant-time compare. Kept separate from the HTTP-rendering layer
/// so the policy can be unit-tested without spinning up a router.
#[derive(Debug)]
pub(crate) enum Decision {
    /// 403 — token state is invalid (missing cookie / missing token).
    Reject(&'static str),
    /// 413 — body too big to buffer for token extraction.
    Oversize,
    /// Run `csrf::verify` against the carried tokens.
    Verify { form: String, cookie: String },
}

/// Axum 0.7 middleware. Mounted globally after `auth_middleware`. Takes
/// `State<AppState>` so it can read the configured cookie name and signing
/// key without reaching into request extensions.
pub async fn csrf_verify_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, AppError> {
    if !is_state_changing(req.method()) {
        return Ok(next.run(req).await);
    }
    let path = req.uri().path().to_string();
    if is_csrf_exempt(&path) {
        return Ok(next.run(req).await);
    }
    let user_id = req
        .extensions()
        .get::<CurrentUser>()
        .map(|cu| cu.id.clone());
    let method = req.method().clone();
    let (parts, body) = req.into_parts();

    let (decision, body) = decide(&parts.headers, body, &state.config.csrf_cookie_name).await;
    match decision {
        Decision::Oversize => Ok(reject_oversize(&method, &path, &user_id)),
        Decision::Reject(reason) => Ok(reject(&method, &path, &user_id, reason)),
        Decision::Verify { form, cookie } => {
            if csrf::verify(&form, &cookie, state.session_key.as_slice()).is_err() {
                return Ok(reject(&method, &path, &user_id, "csrf.token_mismatch"));
            }
            Ok(next.run(Request::from_parts(parts, body)).await)
        }
    }
}

/// Pure-ish (modulo body buffering) decision over a request's headers
/// and body. Returns the (possibly reconstructed) body so the caller can
/// hand it back to the next layer when verification succeeds.
pub(crate) async fn decide(headers: &HeaderMap, body: Body, cookie_name: &str) -> (Decision, Body) {
    let Some(cookie) = read_cookie_token(headers, cookie_name) else {
        return (Decision::Reject("csrf.cookie_missing"), body);
    };
    let header_token = read_header_token(headers);
    let (form_token, body) =
        match resolve_form_token(header_token, headers, body, MAX_BUFFERED_BODY).await {
            Ok(v) => v,
            Err(()) => return (Decision::Oversize, Body::empty()),
        };
    match form_token {
        Some(form) => (Decision::Verify { form, cookie }, body),
        None => (Decision::Reject("csrf.token_missing"), body),
    }
}

/// CSRF is checked on every body-bearing verb. Anything else (`GET`,
/// `HEAD`, `OPTIONS`) is a read and skipped.
pub(crate) fn is_state_changing(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

/// Allowlist for routes that don't yet have a `__Host-csrf` cookie (pre-
/// auth) or aren't user-driven. `/logout` is intentionally NOT here — the
/// user has the cookie by the time they sign out.
pub(crate) fn is_csrf_exempt(path: &str) -> bool {
    matches!(path, "/login" | "/health") || path.starts_with("/static/")
}

fn read_cookie_token(headers: &HeaderMap, cookie_name: &str) -> Option<String> {
    let jar = CookieJar::from_headers(headers);
    jar.get(cookie_name).map(|c| c.value().to_string())
}

fn read_header_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(CSRF_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Header wins when present; otherwise, if the request is form-urlencoded
/// (not multipart, not JSON), buffer up to `cap` bytes and parse out the
/// `csrf_token` field. Returns the reconstructed body for downstream use.
async fn resolve_form_token(
    header_token: Option<String>,
    headers: &HeaderMap,
    body: Body,
    cap: usize,
) -> Result<(Option<String>, Body), ()> {
    if let Some(t) = header_token {
        return Ok((Some(t), body));
    }
    if !is_form_urlencoded(headers) {
        // Multipart (audio) and JSON paths must use the header. No buffering.
        return Ok((None, body));
    }
    let bytes = to_bytes(body, cap).await.map_err(|_| ())?;
    let token = extract_form_field(&bytes, CSRF_FORM_FIELD);
    Ok((token, Body::from(bytes)))
}

fn is_form_urlencoded(headers: &HeaderMap) -> bool {
    headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_ascii_lowercase().starts_with(FORM_CONTENT_TYPE))
        .unwrap_or(false)
}

pub(crate) fn extract_form_field(bytes: &[u8], field: &str) -> Option<String> {
    // `serde_urlencoded` parses into a `Vec<(String, String)>` for the
    // duplicate-tolerant case; the first match for our field wins. Direct
    // dependency via axum-extra's form feature, no extra crate needed.
    let pairs: Vec<(String, String)> = serde_urlencoded::from_bytes(bytes).ok()?;
    pairs
        .into_iter()
        .find(|(k, _)| k == field)
        .map(|(_, v)| v)
        .filter(|v| !v.is_empty())
}

/// Emit a structured log line and return a 403. Token values are NEVER
/// included — only the rejection reason, method, path, and (when known)
/// the authenticated user_id.
fn reject(method: &Method, path: &str, user_id: &Option<String>, event: &'static str) -> Response {
    let uid = user_id.clone().unwrap_or_default();
    tracing::warn!(
        event = event,
        method = %method,
        path = %path,
        user_id = %uid,
        "csrf rejected"
    );
    (StatusCode::FORBIDDEN, "CSRF check failed").into_response()
}

/// 413 specifically when the buffered-body cap is exceeded. Kept separate
/// from the 403 path because the failure mode is "too big to verify",
/// not "verification failed".
fn reject_oversize(method: &Method, path: &str, user_id: &Option<String>) -> Response {
    let uid = user_id.clone().unwrap_or_default();
    tracing::warn!(
        event = "csrf.body_too_large",
        method = %method,
        path = %path,
        user_id = %uid,
        cap_bytes = MAX_BUFFERED_BODY,
        "csrf body cap exceeded"
    );
    (
        StatusCode::PAYLOAD_TOO_LARGE,
        "Request body exceeds CSRF verification cap",
    )
        .into_response()
}

#[cfg(test)]
#[path = "csrf_verify_tests.rs"]
mod tests;

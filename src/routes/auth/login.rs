//! `/login` — render form + accept invite code, mint session cookie.
//!
//! Flow:
//!   1. Throttle by remote IP (window-based failure counter).
//!   2. Cheaply reject malformed codes BEFORE argon2 verify — otherwise
//!      the endpoint becomes a DoS amplifier (verify costs ~50 ms).
//!   3. Scan active users and verify the code against each. First match
//!      wins. O(N) is fine at preview scale.
//!   4. On match: stamp activated_at, mint signed session cookie + CSRF
//!      cookie, 303 to `/`.
//!   5. On no match: record the failure and return 401 with a generic
//!      "Invalid code" — never leaks which user / code prefix.

use askama::Template;
use axum::extract::State;
use axum::http::{header::HeaderName, HeaderMap, HeaderValue, Request, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use axum_extra::extract::Form;
use chrono::Utc;
use serde::Deserialize;
use std::net::{IpAddr, Ipv4Addr};

use crate::error::AppError;
use crate::filters; // base.html's sidebar footer calls custom filters.
use crate::models::User;
use crate::services::{code_hash, cookies, csrf, invite_codes, user_store};
use crate::startup::AppState;

const SET_COOKIE: HeaderName = HeaderName::from_static("set-cookie");
const HX_REDIRECT: HeaderName = HeaderName::from_static("hx-redirect");

#[derive(Template)]
#[template(path = "auth/login.html")]
struct LoginTemplate {
    error: Option<String>,
    csrf_token: String,
    code_max_len: usize,
}

#[derive(Deserialize)]
pub struct LoginForm {
    pub code: String,
    /// CSRF token from the hidden input. `POST /login` is on the
    /// `csrf_verify_middleware` allowlist because no `__Host-csrf`
    /// cookie exists pre-auth — the field is bound only so axum's Form
    /// extractor doesn't reject extra body fields. The minted token is
    /// still rendered so a single-page-app login (which would race the
    /// cookie set) has somewhere to read it from.
    #[serde(default)]
    #[allow(dead_code)]
    pub csrf_token: String,
}

/// `GET /login` — render the form, or 303 to `/` if already signed in.
pub(super) async fn show(
    State(state): State<AppState>,
    req: Request<axum::body::Body>,
) -> Result<Response, AppError> {
    if is_already_signed_in(&state, &req) {
        return Ok(Redirect::to("/").into_response());
    }
    let csrf_token = csrf::mint("anon", state.session_key.as_slice());
    render_login(&state, None, csrf_token, StatusCode::OK)
}

/// `POST /login` — validate format → throttle → scan + verify → cookie + 303.
pub(super) async fn submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Result<Response, AppError> {
    let ip = client_ip(&headers);
    let code = form.code.trim().to_string();
    let prefix = invite_codes::prefix(&code).to_string();

    state
        .login_throttle
        .check(
            &ip,
            state.config.login_throttle_max_attempts,
            state.config.login_throttle_window_secs,
        )
        .await?;

    if !invite_codes::is_valid_format(&code) {
        log_login(false, &prefix, &ip, "bad_format");
        let csrf_token = csrf::mint("anon", state.session_key.as_slice());
        return render_login(
            &state,
            Some("Invalid code format".into()),
            csrf_token,
            StatusCode::BAD_REQUEST,
        );
    }

    let user_match = find_user_by_code(&state, &code).await?;
    match user_match {
        Some(user) => issue_session(&state, user, &ip, &prefix).await,
        None => reject_login(&state, &ip, &prefix).await,
    }
}

/// Iterate active users and argon2-verify each. O(N) is intentional —
/// at preview scale (single digits) this is dominated by verify cost,
/// not the linear scan. Tightening would require a code-derived lookup
/// key, which leaks invariant about the code shape.
async fn find_user_by_code(state: &AppState, code: &str) -> Result<Option<User>, AppError> {
    for user in user_store::list_active_for_login(&state.pool).await? {
        if code_hash::verify(code, &user.code_hash) {
            return Ok(Some(user));
        }
    }
    Ok(None)
}

/// Mint cookies + 303. Best-effort stamps `activated_at` on first login
/// — the DB op is non-gating because the cookie is what unlocks the user.
async fn issue_session(
    state: &AppState,
    user: User,
    ip: &IpAddr,
    prefix: &str,
) -> Result<Response, AppError> {
    if user.activated_at.is_none() {
        let _ = user_store::set_activated_at(&state.pool, &user.id, Utc::now()).await;
    }
    let _ = state.login_throttle.record_success(ip, prefix).await;

    let token = cookies::sign(
        &user.id,
        Utc::now().timestamp(),
        state.session_key.as_slice(),
    );
    let csrf_token = csrf::mint(&user.id, state.session_key.as_slice());
    let cookies_value = build_login_cookies(state, &token, &csrf_token);

    log_login(true, prefix, ip, "ok");

    let mut resp = Redirect::to("/").into_response();
    let headers = resp.headers_mut();
    for v in cookies_value {
        if let Ok(hv) = HeaderValue::from_str(&v) {
            headers.append(SET_COOKIE, hv);
        }
    }
    // HTMX submit: instruct the client to do a top-level navigation.
    if crate::middleware::current_is_htmx() {
        if let Ok(hv) = HeaderValue::from_str("/") {
            headers.insert(HX_REDIRECT, hv);
        }
    }
    Ok(resp)
}

async fn reject_login(state: &AppState, ip: &IpAddr, prefix: &str) -> Result<Response, AppError> {
    let _ = state.login_throttle.record_failure(ip, prefix).await;
    log_login(false, prefix, ip, "no_match");
    let csrf_token = csrf::mint("anon", state.session_key.as_slice());
    render_login(
        state,
        Some("Invalid code".into()),
        csrf_token,
        StatusCode::UNAUTHORIZED,
    )
}

fn build_login_cookies(state: &AppState, session: &str, csrf: &str) -> Vec<String> {
    let dev = state.config.dev_insecure;
    let window = state.config.login_throttle_window_secs.max(60);
    vec![
        super::session_set_cookie(&state.config.session_cookie_name, session, window, dev),
        super::csrf_set_cookie(&state.config.csrf_cookie_name, csrf, window, dev),
    ]
}

fn render_login(
    _state: &AppState,
    error: Option<String>,
    csrf_token: String,
    status: StatusCode,
) -> Result<Response, AppError> {
    let html = LoginTemplate {
        error,
        csrf_token,
        code_max_len: invite_codes::CODE_LEN,
    }
    .render()
    .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok((status, Html(html)).into_response())
}

/// True iff the request already carries a valid session cookie. Used by
/// `GET /login` to short-circuit to `/`. Misses (revoked, expired,
/// missing) silently fall through to the form — `authenticate` already
/// logs them, and the user lands on the right page either way.
fn is_already_signed_in(state: &AppState, req: &Request<axum::body::Body>) -> bool {
    let jar = CookieJar::from_headers(req.headers());
    let Some(cookie) = jar.get(&state.config.session_cookie_name) else {
        return false;
    };
    let max_age = i64::try_from(state.config.login_throttle_window_secs).unwrap_or(3600);
    cookies::verify(
        cookie.value(),
        state.session_key.as_slice(),
        max_age,
        Utc::now().timestamp(),
    )
    .is_ok()
}

/// Best-effort remote-IP for throttle keying. Order: `X-Forwarded-For`
/// (first hop, used when a reverse proxy fronts the app) → `X-Real-IP`
/// → loopback fallback. Never trust the header for authorization —
/// only for forensic + rate-limit bucketing. Avoiding `ConnectInfo`
/// keeps the `axum::serve` setup simple; in production the reverse
/// proxy must be configured to set XFF (the default for `nginx`,
/// `caddy`, `traefik`, and `cloudflared`).
fn client_ip(headers: &HeaderMap) -> IpAddr {
    if let Some(forwarded) = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim())
        .and_then(|s| s.parse::<IpAddr>().ok())
    {
        return forwarded;
    }
    if let Some(real) = headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<IpAddr>().ok())
    {
        return real;
    }
    IpAddr::V4(Ipv4Addr::LOCALHOST)
}

/// One audit log line per attempt. Plain code is never logged; only the
/// first-4 (`code_prefix`) — matches `login_attempts.code_prefix`.
fn log_login(success: bool, code_prefix: &str, ip: &IpAddr, reason: &str) {
    tracing::info!(
        event = "auth.login",
        success = success,
        code_prefix = %code_prefix,
        remote_ip = %ip,
        reason = reason,
        "login attempt"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn header_with(name: &str, value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            HeaderName::from_lowercase(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
        h
    }

    #[test]
    fn case_client_ip_prefers_x_forwarded_for_first_hop() {
        let h = header_with("x-forwarded-for", "203.0.113.5, 198.51.100.7");
        assert_eq!(client_ip(&h).to_string(), "203.0.113.5");
    }

    #[test]
    fn case_client_ip_uses_x_real_ip_when_xff_missing() {
        let h = header_with("x-real-ip", "203.0.113.9");
        assert_eq!(client_ip(&h).to_string(), "203.0.113.9");
    }

    #[test]
    fn case_client_ip_falls_back_to_loopback() {
        let h = HeaderMap::new();
        assert_eq!(client_ip(&h), IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn case_client_ip_ignores_malformed_header() {
        let h = header_with("x-forwarded-for", "not-an-ip");
        assert_eq!(client_ip(&h), IpAddr::V4(Ipv4Addr::LOCALHOST));
    }
}

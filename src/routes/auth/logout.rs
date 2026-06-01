//! `/logout` — clear cookies + redirect to `/login`.
//!
//! CSRF double-submit will be wired in Phase 2; for now we accept the
//! field but don't reject on mismatch. Even a forged GET → `/logout` is
//! a low-impact action (the user gets signed out), so the gap is small.

use axum::extract::State;
use axum::http::{header::HeaderName, HeaderValue};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use axum_extra::extract::Form;
use serde::Deserialize;

use crate::error::AppError;
use crate::services::cookies;
use crate::startup::AppState;

const SET_COOKIE: HeaderName = HeaderName::from_static("set-cookie");

#[derive(Deserialize)]
pub struct LogoutForm {
    #[serde(default)]
    #[allow(dead_code)] // TODO(phase-2): wire double-submit verify.
    pub csrf_token: String,
}

pub(super) async fn submit(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(_form): Form<LogoutForm>,
) -> Result<Response, AppError> {
    log_logout(&state, &jar);

    let mut resp = Redirect::to("/login").into_response();
    let headers = resp.headers_mut();
    for v in build_expire_cookies(&state) {
        if let Ok(hv) = HeaderValue::from_str(&v) {
            headers.append(SET_COOKIE, hv);
        }
    }
    Ok(resp)
}

fn build_expire_cookies(state: &AppState) -> Vec<String> {
    let dev = state.config.dev_insecure;
    vec![
        super::expired_cookie(&state.config.session_cookie_name, true, dev),
        super::expired_cookie(&state.config.csrf_cookie_name, false, dev),
    ]
}

/// Decode the existing session cookie just enough to log the signing
/// out user_id. Verification failures fall through to an anonymous log
/// — the user is logging out either way, no point gating on the cookie.
fn log_logout(state: &AppState, jar: &CookieJar) {
    let user_id = jar
        .get(&state.config.session_cookie_name)
        .and_then(|c| {
            cookies::verify(
                c.value(),
                state.session_key.as_slice(),
                i64::try_from(state.config.login_throttle_window_secs).unwrap_or(3600),
                chrono::Utc::now().timestamp(),
            )
            .ok()
        })
        .unwrap_or_default();
    tracing::info!(
        event = "auth.logout",
        user_id = %user_id,
        "user signed out"
    );
}

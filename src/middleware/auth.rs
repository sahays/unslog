//! Global auth middleware.
//!
//! Runs after `request_context_middleware` so the request_id is already set
//! when we log auth events. Reads the `__Host-session` cookie, validates it
//! via `services::auth::authenticate`, and inserts a [`CurrentUser`] into
//! request extensions. The current user's label + a freshly-minted CSRF
//! token are also published into task-locals so `base.html`'s sidebar
//! footer can read them without each template having to thread the values
//! through its own struct field (Phase 2 sweep will move handlers onto the
//! extension).
//!
//! Allowlist (no cookie required): `/login`, `/logout`, `/static/*`,
//! `/health`. Everything else 303s to `/login` on a missing/invalid cookie,
//! or surfaces `HX-Redirect: /login` for HTMX swaps so the browser does a
//! top-level navigation instead of injecting the login page into a fragment.

use axum::body::Body;
use axum::extract::State;
use axum::http::{header::HeaderName, HeaderValue, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use std::sync::Arc;

use crate::services::auth::{self, CurrentUser};
use crate::services::csrf;
use crate::startup::AppState;

const HX_REDIRECT: HeaderName = HeaderName::from_static("hx-redirect");
const LOGIN_PATH: &str = "/login";

tokio::task_local! {
    /// Display label of the logged-in user. Read by `base.html` via the
    /// `current_user_label` Askama filter so per-template structs don't have
    /// to carry the field everywhere in Phase 1.2.
    pub static CURRENT_USER_LABEL: String;
    /// Freshly-minted CSRF token for the in-flight request. Read by the
    /// `csrf_token` filter for the sidebar sign-out form.
    pub static CURRENT_CSRF_TOKEN: String;
    /// True when the in-flight request's user has `is_master = true`. Read
    /// by `base.html` via the `current_user_is_master` Askama filter so
    /// admin-only sidebar links can be hidden for non-master accounts.
    pub static CURRENT_USER_IS_MASTER: bool;
}

/// Best-effort lookup of the current user's label. Empty when no user is
/// signed in (e.g. on `/login`) — safe default for template rendering.
pub fn current_user_label() -> String {
    CURRENT_USER_LABEL
        .try_with(|v| v.clone())
        .unwrap_or_default()
}

/// Best-effort lookup of the current request's CSRF token. Empty outside
/// a request scope.
pub fn current_csrf_token() -> String {
    CURRENT_CSRF_TOKEN
        .try_with(|v| v.clone())
        .unwrap_or_default()
}

/// Best-effort lookup of the current user's master flag. Defaults to
/// `false` outside a request scope (so admin-only template fragments stay
/// hidden when there's no signed-in user).
pub fn current_user_is_master() -> bool {
    CURRENT_USER_IS_MASTER.try_with(|v| *v).unwrap_or(false)
}

/// Axum 0.7 middleware closure. Mounted globally; per-path exemption is
/// handled here rather than via a sub-router so the allowlist is one
/// readable list instead of two `Router::merge` sites.
pub async fn auth_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();
    if is_allowlisted(&path) {
        return next.run(request).await;
    }

    let jar = CookieJar::from_headers(request.headers());
    let cookie_value = jar
        .get(&state.config.session_cookie_name)
        .map(|c| c.value().to_string());
    let existing_csrf = jar
        .get(&state.config.csrf_cookie_name)
        .map(|c| c.value().to_string());

    let is_htmx = crate::middleware::current_is_htmx();
    let cookie_value = match cookie_value {
        Some(v) => v,
        None => return unauthenticated_redirect(is_htmx),
    };

    match auth::authenticate(
        &cookie_value,
        state.session_key.as_slice(),
        i64::try_from(state.config.login_throttle_window_secs).unwrap_or(3600),
        &*state.auth_deps,
    )
    .await
    {
        Ok(cu) => attach_and_continue(cu, &state, existing_csrf, request, next).await,
        Err(_) => unauthenticated_redirect(is_htmx),
    }
}

/// Decide whether a path bypasses auth. Static + health + the auth pages
/// themselves; everything else is gated.
fn is_allowlisted(path: &str) -> bool {
    matches!(path, "/login" | "/logout" | "/health")
        || path.starts_with("/static/")
        || path.starts_with("/recordings/")
}

/// Stamp the resolved user onto the request, publish label + CSRF token +
/// is_master into task-locals, and run the rest of the chain inside that
/// scope.
///
/// CSRF token comes from the existing `__Host-csrf` cookie when present —
/// that's the source of truth for the double-submit pattern, so every form
/// rendered during this request gets a token byte-identical to the cookie
/// `csrf_verify_middleware` will compare against. When the cookie is
/// missing (first authenticated page after login expired the cookie, or a
/// browser session that lost it), we mint a fresh token and queue a
/// `Set-Cookie` on the response so subsequent requests see them in sync.
async fn attach_and_continue(
    cu: CurrentUser,
    state: &AppState,
    existing_csrf: Option<String>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let label = cu.label.clone();
    let is_master = cu.is_master;
    let (csrf_token, refresh_cookie) = match existing_csrf {
        Some(v) => (v, false),
        None => (csrf::mint(&cu.id, state.session_key.as_slice()), true),
    };
    let cookie_to_set = if refresh_cookie {
        Some(crate::routes::auth::csrf_set_cookie(
            &state.config.csrf_cookie_name,
            &csrf_token,
            state.config.login_throttle_window_secs,
            state.config.dev_insecure,
        ))
    } else {
        None
    };
    request.extensions_mut().insert(cu);
    let mut response = CURRENT_USER_LABEL
        .scope(
            label,
            CURRENT_CSRF_TOKEN.scope(
                csrf_token,
                CURRENT_USER_IS_MASTER.scope(is_master, next.run(request)),
            ),
        )
        .await;
    if let Some(value) = cookie_to_set {
        if let Ok(hv) = HeaderValue::from_str(&value) {
            response
                .headers_mut()
                .append(axum::http::header::SET_COOKIE, hv);
        }
    }
    response
}

fn unauthenticated_redirect(is_htmx: bool) -> Response {
    if is_htmx {
        let mut resp = (StatusCode::UNAUTHORIZED, "Sign in required").into_response();
        if let Ok(hv) = HeaderValue::from_str(LOGIN_PATH) {
            resp.headers_mut().insert(HX_REDIRECT, hv);
        }
        resp
    } else {
        Redirect::to(LOGIN_PATH).into_response()
    }
}

/// Stand-alone helper used by the route layer to read the CurrentUser
/// extension; isolates the extension key from individual handlers.
#[allow(dead_code)]
pub fn current_user_from(req: &Request<Body>) -> Option<Arc<CurrentUser>> {
    req.extensions().get::<Arc<CurrentUser>>().cloned()
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;

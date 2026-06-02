//! `/login` (GET, POST) and `/logout` (POST).
//!
//! The auth middleware allow-lists these three exact endpoints; everything
//! else 303s here when the session cookie is missing or invalid. Split:
//! * `login` — form render + credential check + cookie mint.
//! * `logout` — clear cookie + redirect.
//!
//! Shared helpers (cookie attribute string builders) sit here.

use axum::routing::{get, post};
use axum::Router;

use crate::startup::AppState;

mod login;
mod logout;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/login", get(login::show).post(login::submit))
        .route("/logout", post(logout::submit))
}

/// Build a `Set-Cookie` value for the session token. Always
/// `HttpOnly; SameSite=Strict; Path=/; Max-Age=<window>`. `Secure` is
/// emitted unless `dev_insecure` is true — keeps localhost http://
/// development working without weakening the production attribute set.
pub(crate) fn session_set_cookie(
    name: &str,
    value: &str,
    max_age: u64,
    dev_insecure: bool,
) -> String {
    build_cookie(name, value, max_age, true, dev_insecure)
}

/// Build a `Set-Cookie` value for the CSRF token. Same attributes as the
/// session cookie except `HttpOnly` is OFF — HTMX needs to read it for the
/// double-submit pattern.
pub(crate) fn csrf_set_cookie(name: &str, value: &str, max_age: u64, dev_insecure: bool) -> String {
    build_cookie(name, value, max_age, false, dev_insecure)
}

/// Build an expiring `Set-Cookie` value (Max-Age=0) for the named cookie.
/// Used by `/logout` to clear both session + csrf in one response.
pub(crate) fn expired_cookie(name: &str, http_only: bool, dev_insecure: bool) -> String {
    build_cookie(name, "", 0, http_only, dev_insecure)
}

fn build_cookie(
    name: &str,
    value: &str,
    max_age: u64,
    http_only: bool,
    dev_insecure: bool,
) -> String {
    let mut s = format!("{name}={value}; Path=/; SameSite=Strict; Max-Age={max_age}");
    if http_only {
        s.push_str("; HttpOnly");
    }
    if !dev_insecure {
        s.push_str("; Secure");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_session_cookie_has_httponly_and_secure_by_default() {
        let c = session_set_cookie("__Host-session", "abc", 3600, false);
        assert!(c.contains("HttpOnly"));
        assert!(c.contains("Secure"));
        assert!(c.contains("SameSite=Strict"));
        assert!(c.contains("Path=/"));
        assert!(c.contains("Max-Age=3600"));
    }

    #[test]
    fn case_session_cookie_drops_secure_when_dev_insecure() {
        let c = session_set_cookie("__Host-session", "abc", 3600, true);
        assert!(!c.contains("Secure"));
        assert!(c.contains("HttpOnly"));
    }

    #[test]
    fn case_csrf_cookie_omits_httponly() {
        let c = csrf_set_cookie("__Host-csrf", "tok", 3600, false);
        assert!(!c.contains("HttpOnly"));
        assert!(c.contains("Secure"));
        assert!(c.contains("SameSite=Strict"));
    }

    #[test]
    fn case_expired_cookie_uses_zero_max_age() {
        let c = expired_cookie("__Host-session", true, false);
        assert!(c.contains("Max-Age=0"));
        assert!(c.contains("HttpOnly"));
    }
}

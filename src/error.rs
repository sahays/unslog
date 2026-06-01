use askama::Template;
use axum::http::{header::HeaderName, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum_htmx::HxRequest;

/// Path the browser should be sent to when an unauthenticated session hits a
/// protected resource. `/login` is wired up in Phase 1; the variant is
/// introduced here so callers can already start returning it.
const LOGIN_PATH: &str = "/login";
const HX_REDIRECT: HeaderName = HeaderName::from_static("hx-redirect");

#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("openrouter not configured (set OPENROUTER_API_KEY)")]
    OpenRouterNotConfigured,

    /// No (or invalid) session cookie on a protected route. Browsers get a
    /// 303 redirect to `/login`; HTMX swaps get a 401 + `HX-Redirect` so the
    /// client triggers a top-level navigation instead of replacing the
    /// fragment with a login page.
    #[error("sign-in required")]
    Unauthenticated,

    /// Authenticated, but the resource isn't owned/permitted for this user.
    /// Always renders the standard error chrome — never reveals existence.
    #[error("not allowed")]
    Forbidden,

    /// Caller has burned through their daily request budget. `used` and
    /// `cap` are inlined into the body so the user sees their actual cap;
    /// they are logged as structured fields, never PII.
    #[error("You've used {used} of {cap} requests this period.")]
    RateLimited { used: u32, cap: u32 },

    #[error("upstream error: {0}")]
    Upstream(String),

    #[error("database error")]
    Sqlx(#[from] sqlx::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Http(#[from] reqwest::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl AppError {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::OpenRouterNotConfigured => StatusCode::SERVICE_UNAVAILABLE,
            Self::Unauthenticated => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// User-facing label that names the kind of failure without leaking the
    /// raw message. Pairs with the detailed message inside the error body.
    fn kind_label(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "Not found",
            Self::BadRequest(_) => "Bad request",
            Self::OpenRouterNotConfigured => "OpenRouter not configured",
            Self::Unauthenticated => "Sign in required",
            Self::Forbidden => "Not allowed",
            Self::RateLimited { .. } => "Rate limit reached",
            Self::Upstream(_) => "Upstream service error",
            Self::Sqlx(_) => "Database error",
            Self::Io(_) => "I/O error",
            Self::Http(_) => "Network error",
            Self::Json(_) => "Bad data",
            Self::Other(_) => "Server error",
        }
    }
}

#[derive(Template)]
#[template(path = "errors/error.html")]
struct ErrorTemplate<'a> {
    status: u16,
    kind_label: &'a str,
    message: &'a str,
    request_id: &'a str,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        let kind_label = self.kind_label();
        let message = self.to_string();
        let request_id = crate::middleware::current_request_id();
        let is_htmx = crate::middleware::current_is_htmx();

        log_error(&self, status, &message);

        // `Unauthenticated` is the only variant that does NOT render the
        // standard error chrome — it redirects so the user lands on /login.
        if matches!(self, Self::Unauthenticated) {
            return unauthenticated_response(is_htmx);
        }

        if is_htmx {
            return htmx_error(status, kind_label, &message);
        }
        full_page_error(status, kind_label, &message, &request_id)
    }
}

/// Tracing severity mapping for `AppError`. Server faults stay `error`,
/// upstream-unavailable is `warn`, rate-limit is `warn`, auth events are
/// `info` (routine), and other 4xx is `info`. Error messages can carry
/// user-supplied substrings, so the preview is truncated before logging.
fn log_error(err: &AppError, status: StatusCode, message: &str) {
    let preview = crate::services::redact::preview(message, 200);
    let code = status.as_u16();
    match err {
        AppError::Unauthenticated => {
            tracing::info!(
                event = "error.unauthenticated",
                status = code,
                "auth required"
            );
        }
        AppError::Forbidden => {
            tracing::info!(event = "error.forbidden", status = code, "forbidden");
        }
        AppError::RateLimited { used, cap } => {
            tracing::warn!(
                event = "error.rate_limited",
                status = code,
                used = used,
                cap = cap,
                "rate limit reached"
            );
        }
        _ if status.is_server_error() => {
            tracing::error!(error = %preview, status = code, "request failed");
        }
        _ if status == StatusCode::SERVICE_UNAVAILABLE => {
            tracing::warn!(error = %preview, status = code, "request unavailable");
        }
        _ => {
            tracing::info!(error = %preview, status = code, "request rejected");
        }
    }
}

/// HTMX-aware unauthenticated response. Full-page browsers get a 303 to
/// `/login`; HTMX swaps get a 401 + `HX-Redirect` so the client triggers a
/// top-level navigation rather than replacing the fragment.
fn unauthenticated_response(is_htmx: bool) -> Response {
    if is_htmx {
        let mut resp = htmx_error(
            StatusCode::UNAUTHORIZED,
            "Sign in required",
            "Redirecting to sign-in…",
        );
        if let Ok(hv) = HeaderValue::from_str(LOGIN_PATH) {
            resp.headers_mut().insert(HX_REDIRECT, hv);
        }
        resp
    } else {
        axum::response::Redirect::to(LOGIN_PATH).into_response()
    }
}

/// Small HTML fragment for HTMX swaps. Body is HTML-escaped because LLM
/// output / user input can flow into error messages.
pub fn htmx_error(status: StatusCode, kind_label: &str, message: &str) -> Response {
    let body = format!(
        r#"<div class="alert alert-error" role="alert"><strong>{}</strong><div class="muted">{}</div></div>"#,
        html_escape(kind_label),
        html_escape(message)
    );
    (status, Html(body)).into_response()
}

fn full_page_error(
    status: StatusCode,
    kind_label: &str,
    message: &str,
    request_id: &str,
) -> Response {
    let html = ErrorTemplate {
        status: status.as_u16(),
        kind_label,
        message,
        request_id,
    }
    .render()
    // Defensive fallback: a render failure inside the error path
    // shouldn't bubble into a second 500. Drop to a tiny inline card.
    .unwrap_or_else(|_| {
        format!(
            r#"<div class="card" style="border-color: var(--color-error);"><strong>{kind_label}</strong><div class="muted" style="margin-top:.5rem">{}</div></div>"#,
            html_escape(message)
        )
    });
    (status, Html(html)).into_response()
}

#[derive(Template)]
#[template(path = "errors/404.html")]
struct NotFoundTemplate;

pub async fn not_found_handler(HxRequest(is_htmx): HxRequest) -> Response {
    if is_htmx {
        (
            StatusCode::NOT_FOUND,
            Html(r#"<div class="card">Not found.</div>"#.to_string()),
        )
            .into_response()
    } else {
        let html = NotFoundTemplate
            .render()
            .unwrap_or_else(|_| "Not Found".into());
        (StatusCode::NOT_FOUND, Html(html)).into_response()
    }
}

pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Render an Askama template into an `Html<String>` response, mapping
/// template-render failures to a uniform `AppError::Other`. Centralizing
/// this here keeps route handlers from sprinkling
/// `.render().map_err(|e| AppError::Other(anyhow::anyhow!(e)))?` boilerplate
/// at every site.
pub fn render_html<T: askama::Template>(tmpl: T) -> Result<Html<String>, AppError> {
    Ok(Html(
        tmpl.render()
            .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?,
    ))
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;

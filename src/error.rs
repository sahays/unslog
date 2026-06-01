use askama::Template;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum_htmx::HxRequest;

#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("openrouter not configured (set OPENROUTER_API_KEY)")]
    OpenRouterNotConfigured,

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
    fn status(&self) -> StatusCode {
        match self {
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::OpenRouterNotConfigured => StatusCode::SERVICE_UNAVAILABLE,
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

        // Severity tracks status: user-input errors (4xx) shouldn't pollute
        // the error stream, but server faults (5xx) and the OpenRouter-not-
        // configured case (503) deserve attention. Error messages can carry
        // interpolated user input (e.g. "unknown role {form.role}"), so we
        // truncate the rendered display before it lands on disk.
        let error_preview = crate::services::redact::preview(&message, 200);
        if status.is_server_error() {
            tracing::error!(error = %error_preview, status = status.as_u16(), "request failed");
        } else if status == StatusCode::SERVICE_UNAVAILABLE {
            tracing::warn!(error = %error_preview, status = status.as_u16(), "request unavailable");
        } else {
            tracing::info!(error = %error_preview, status = status.as_u16(), "request rejected");
        }

        let html = ErrorTemplate {
            status: status.as_u16(),
            kind_label,
            message: &message,
            request_id: &request_id,
        }
        .render()
        // Defensive fallback: a render failure inside the error path
        // shouldn't bubble into a second 500. Drop to a tiny inline card.
        .unwrap_or_else(|_| {
            format!(
                r#"<div class="card" style="border-color: var(--color-error);"><strong>{kind_label}</strong><div class="muted" style="margin-top:.5rem">{}</div></div>"#,
                html_escape(&message)
            )
        });
        (status, Html(html)).into_response()
    }
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

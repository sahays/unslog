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

    #[error(transparent)]
    Mongo(#[from] mongodb::error::Error),

    #[error(transparent)]
    Bson(#[from] bson::ser::Error),

    #[error(transparent)]
    BsonDe(#[from] bson::de::Error),

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
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!(error = %self, "request failed");
        let status = self.status();
        let msg = self.to_string();
        let body = format!(
            r#"<div class="card" style="border-color: var(--danger);"><strong>Error</strong><div class="muted" style="margin-top:.5rem">{}</div></div>"#,
            html_escape(&msg)
        );
        (status, Html(body)).into_response()
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
        let html = NotFoundTemplate.render().unwrap_or_else(|_| "Not Found".into());
        (StatusCode::NOT_FOUND, Html(html)).into_response()
    }
}

pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

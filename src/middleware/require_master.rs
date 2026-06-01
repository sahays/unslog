//! Master-only authorization gate.
//!
//! Mounted by the route layer on admin surfaces (settings, prompts/agents,
//! and any other route that mutates global content). Runs *after* the
//! global auth middleware, so the `CurrentUser` extension is already
//! populated. Non-master callers get `AppError::Forbidden` (403).
//!
//! Resume-asset writes are NOT gated here — every signed-in user can
//! upload their own resume. Book-asset writes are gated in the handler
//! via `services::assets::ensure_book_mutable_by`.

use axum::body::Body;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;

use crate::error::AppError;
use crate::services::auth::CurrentUser;

/// Require the request's `CurrentUser` to be `is_master`. Logs every
/// rejection as `event = "auth.forbidden"` with the route + user id so
/// non-master access attempts on admin pages are auditable.
pub async fn require_master_middleware(
    req: Request<Body>,
    next: Next,
) -> Result<Response, AppError> {
    let user = req
        .extensions()
        .get::<CurrentUser>()
        .cloned()
        .ok_or(AppError::Unauthenticated)?;
    if user.is_master {
        return Ok(next.run(req).await);
    }
    let route = req.uri().path().to_string();
    tracing::info!(
        event = "auth.forbidden",
        user_id = %user.id,
        route = %route,
        "non-master attempted to access master-only route",
    );
    Err(AppError::Forbidden)
}

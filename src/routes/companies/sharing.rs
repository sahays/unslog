//! `/companies/:id/publish` + `/companies/:id/unpublish` — owner-only
//! toggle on the `is_public` flag.
//!
//! Authentication + CSRF are layered globally (`auth_middleware` +
//! `csrf_verify_middleware`); the route handler only needs to call
//! `services::sharing::set_company_public` with the current user's id.
//! The service returns `AppError::NotFound` when the owner_id + id pair
//! doesn't match a row — same response a non-owner would get, which is
//! the existence-leak guard.

use axum::extract::{Extension, Path, State};
use axum::response::{IntoResponse, Redirect, Response};

use crate::error::AppError;
use crate::services::auth::CurrentUser;
use crate::services::sharing;
use crate::startup::AppState;

pub async fn publish(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    sharing::set_company_public(&state.pool, &current_user.id, &id, true).await?;
    tracing::info!(
        event = "share.company.publish",
        company_id = %id,
        owner_id = %current_user.id,
        "company published",
    );
    Ok(Redirect::to(&format!("/companies/{id}")).into_response())
}

pub async fn unpublish(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    sharing::set_company_public(&state.pool, &current_user.id, &id, false).await?;
    tracing::info!(
        event = "share.company.unpublish",
        company_id = %id,
        owner_id = %current_user.id,
        "company unpublished",
    );
    Ok(Redirect::to(&format!("/companies/{id}")).into_response())
}

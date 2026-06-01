//! `POST /settings/invites/:id/revoke` — disable an invite immediately.
//! The auth middleware rejects users whose `revoked_at IS NOT NULL`, so
//! existing cookies stop authenticating on the next request.

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;
use serde::Deserialize;

use crate::error::AppError;
use crate::services::invite_admin::{InviteAdminDeps, PgInviteAdminDeps};
use crate::startup::AppState;

#[derive(Deserialize)]
pub struct RevokeForm {
    #[serde(default)]
    #[allow(dead_code)]
    pub csrf_token: String,
}

pub(super) async fn submit(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Form(_form): Form<RevokeForm>,
) -> Result<Response, AppError> {
    let deps = PgInviteAdminDeps {
        pool: state.pool.clone(),
    };
    deps.revoke(&id).await?;
    Ok(Redirect::to("/settings/invites").into_response())
}

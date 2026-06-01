//! `POST /settings/invites/:id/extend` — push out an invitee's
//! `expires_at`. Always moves the expiry forward (GREATEST in the SQL),
//! never backward.

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;
use serde::Deserialize;

use crate::error::AppError;
use crate::services::invite_admin::{InviteAdminDeps, PgInviteAdminDeps};
use crate::startup::AppState;

#[derive(Deserialize)]
pub struct ExtendForm {
    pub days: u32,
    #[serde(default)]
    #[allow(dead_code)]
    pub csrf_token: String,
}

pub(super) async fn submit(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Form(form): Form<ExtendForm>,
) -> Result<Response, AppError> {
    let deps = PgInviteAdminDeps {
        pool: state.pool.clone(),
    };
    deps.extend(&id, form.days).await?;
    Ok(Redirect::to("/settings/invites").into_response())
}

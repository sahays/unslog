//! `GET /settings/invites` — admin landing for invite management.

use askama::Template;
use axum::extract::State;
use axum::response::Html;

use crate::error::AppError;
use crate::filters; // base.html sidebar uses custom filters.
use crate::services::invite_admin::{InviteAdminDeps, InviteeRow, PgInviteAdminDeps};
use crate::startup::AppState;

#[derive(Template)]
#[template(path = "settings/invites.html")]
struct InvitesTemplate {
    invitees: Vec<InviteeRow>,
    /// Surfaced so the template can show "0 invitees yet" empty-state
    /// without a `.len() == 0` Askama call site that strict mode flags.
    has_invitees: bool,
}

pub(super) async fn show(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let deps = PgInviteAdminDeps {
        pool: state.pool.clone(),
    };
    let invitees = deps.list_invitees().await?;
    crate::error::render_html(InvitesTemplate {
        has_invitees: !invitees.is_empty(),
        invitees,
    })
}

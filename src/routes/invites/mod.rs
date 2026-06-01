//! `/settings/invites/*` — master-only invite-code administration.
//!
//! The endpoints are *merged into* the existing settings router so the
//! `require_master_middleware` layer (mounted in `routes::settings`)
//! gates these handlers too. Nesting a fresh sub-router under
//! `/settings/invites` would not inherit that layer.

use axum::routing::{get, post};
use axum::Router;

use crate::startup::AppState;

mod extend;
mod list;
mod mint;
mod revoke;

/// Routes-only assembly. `routes::settings::routes()` calls this and
/// merges the result into its own Router so all four endpoints inherit
/// the master-only middleware layer.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/settings/invites", get(list::show).post(mint::submit))
        .route("/settings/invites/:id/extend", post(extend::submit))
        .route("/settings/invites/:id/revoke", post(revoke::submit))
}

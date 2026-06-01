//! `/browse/*` — public catalog: see and fork what other users have
//! published. Today: companies. Future: stories, pitches.
//!
//! The browse page does not implement search or filters — preview-scale
//! catalogs are small enough that a flat newest-first list is enough.

use axum::routing::{get, post};
use axum::Router;

use crate::startup::AppState;

mod companies;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/browse", get(companies::index))
        .route("/browse/companies/:id/fork", post(companies::fork))
}

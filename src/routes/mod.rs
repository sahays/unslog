use axum::Router;

use crate::startup::AppState;

pub mod assets;
pub mod health;
pub mod home;
pub mod prompts;

pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(health::routes())
        .merge(home::routes())
        .merge(assets::routes())
        .merge(prompts::routes())
        .with_state(state)
}

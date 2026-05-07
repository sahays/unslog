use axum::Router;

use crate::startup::AppState;

pub mod assets;
pub mod companies;
pub mod health;
pub mod home;
pub mod prompts;
pub mod recordings;
pub mod sessions;

pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(health::routes())
        .merge(home::routes())
        .merge(assets::routes())
        .merge(companies::routes())
        .merge(prompts::routes())
        .merge(recordings::routes())
        .merge(sessions::routes())
        .with_state(state)
}

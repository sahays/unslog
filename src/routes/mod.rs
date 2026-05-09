use axum::Router;

use crate::startup::AppState;

pub mod assets;
pub mod categories;
pub mod companies;
pub mod health;
pub mod home;
pub mod practice;
pub mod prompts;
pub mod recordings;
pub mod sessions;
pub mod settings;
pub mod stories;

pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(health::routes())
        .merge(home::routes())
        .merge(assets::routes())
        .merge(categories::routes())
        .merge(companies::routes())
        .merge(practice::routes())
        .merge(prompts::routes())
        .merge(recordings::routes())
        .merge(sessions::routes())
        .merge(settings::routes())
        .merge(stories::routes())
        .with_state(state)
}

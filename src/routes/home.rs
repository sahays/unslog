use askama::Template;
use axum::{response::Html, routing::get, Router};

use crate::startup::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(index))
}

#[derive(Template)]
#[template(path = "home.html")]
struct HomeTemplate;

async fn index() -> Html<String> {
    Html(HomeTemplate.render().unwrap_or_else(|_| "unslog".into()))
}

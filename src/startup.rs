use axum::extract::DefaultBodyLimit;
use axum::Router;
use mongodb::Database;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

/// Cap upload body at 50 MB. Audio answers and the book PDF both fit comfortably
/// under this; well above that and either we have a very long-winded answer or
/// a misconfigured client.
const MAX_BODY_BYTES: usize = 50 * 1024 * 1024;

use crate::config::AppConfig;
use crate::routes;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub db: Database,
    pub http: reqwest::Client,
}

pub async fn run(config: AppConfig) -> anyhow::Result<()> {
    let db = crate::db::connect(&config.mongo_uri, &config.mongo_db).await?;
    crate::db::ensure_indexes(&db).await?;
    crate::services::prompt_store::seed_defaults(&db).await?;

    tokio::fs::create_dir_all(&config.data_dir).await.ok();
    tokio::fs::create_dir_all(format!("{}/recordings", config.data_dir))
        .await
        .ok();
    tokio::fs::create_dir_all(format!("{}/assets/originals", config.data_dir))
        .await
        .ok();
    tokio::fs::create_dir_all(format!("{}/assets/extracted", config.data_dir))
        .await
        .ok();

    let state = AppState {
        config: Arc::new(config),
        db,
        http: reqwest::Client::new(),
    };

    let static_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/static");
    let app = Router::new()
        .merge(routes::router(state.clone()))
        .nest_service("/static", ServeDir::new(static_dir))
        .fallback(crate::error::not_found_handler)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(TraceLayer::new_for_http());

    let addr = state.config.addr();
    tracing::info!("starting unslog on http://{addr}");
    let listener = TcpListener::bind(&addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received");
}

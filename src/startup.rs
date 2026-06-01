use axum::extract::DefaultBodyLimit;
use axum::middleware::from_fn;
use axum::Router;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::services::ServeDir;

/// Cap upload body at 50 MB. Audio answers and the book PDF both fit comfortably
/// under this; well above that and either we have a very long-winded answer or
/// a misconfigured client.
const MAX_BODY_BYTES: usize = 50 * 1024 * 1024;

use crate::config::AppConfig;
use crate::routes;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    /// Live Postgres pool — the single backing store for the app.
    pub pool: PgPool,
    pub http: reqwest::Client,
    pub openrouter: Arc<dyn crate::services::openrouter::LlmClient>,
    pub models_cache: crate::services::openrouter_models::ModelsCache,
    pub book_cache: crate::services::assets::BookCache,
    pub resume_cache: crate::services::assets::ResumeCache,
    pub settings_cache: crate::services::settings_store::SettingsCache,
    pub prompt_cache: crate::services::prompt_cache::PromptCache,
    /// HMAC-SHA256 server key. Persisted at `<data_dir>/session.key`.
    /// Phase 1.2 will consume this in cookie middleware; carrying it on
    /// `AppState` now so boot fails fast if the file is corrupted.
    #[allow(dead_code)]
    pub session_key: Arc<[u8; 32]>,
}

pub async fn run(config: AppConfig) -> anyhow::Result<()> {
    let pool = crate::services::db::connect_postgres(&config.database_url).await?;
    seed_master_user(&pool, &config).await?;
    crate::services::prompt_store::seed_defaults(&pool).await?;
    crate::services::category_store::seed_defaults(&pool).await?;
    crate::services::pitch_store::seed_defaults(&pool).await?;

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

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| anyhow::anyhow!("reqwest client: {e}"))?;
    let openrouter: Arc<dyn crate::services::openrouter::LlmClient> =
        Arc::new(crate::services::openrouter::OpenRouter::new(
            http.clone(),
            config.openrouter_api_key.clone(),
            config.referer.clone(),
        ));
    // Load (or mint) the server-side HMAC key before any request handlers
    // come up — failing here is preferable to a 500 on the first sign-in.
    let session_key = crate::services::session_key::load_or_create(&config.data_dir).await?;
    let state = AppState {
        config: Arc::new(config),
        pool,
        http,
        openrouter,
        models_cache: crate::services::openrouter_models::ModelsCache::new(),
        book_cache: crate::services::assets::BookCache::new(),
        resume_cache: crate::services::assets::ResumeCache::new(),
        settings_cache: crate::services::settings_store::SettingsCache::new(),
        prompt_cache: crate::services::prompt_cache::PromptCache::new(),
        session_key: Arc::new(session_key),
    };

    let static_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/static");
    let app = Router::new()
        .merge(routes::router(state.clone()))
        .nest_service("/static", ServeDir::new(static_dir))
        .fallback(crate::error::not_found_handler)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(from_fn(crate::middleware::request_context_middleware));

    let addr = state.config.addr();
    tracing::info!(addr = %addr, "starting unslog");
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

/// Hard-fail boot when `MASTER_INVITE_CODE` is missing; otherwise hand
/// off to `master_seed::ensure_master`. Kept out of `run()` so the main
/// boot sequence reads like pseudocode.
async fn seed_master_user(pool: &PgPool, config: &AppConfig) -> anyhow::Result<()> {
    let code = config.master_invite_code.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "MASTER_INVITE_CODE not set in .env (12 alphanumerics required for first boot)"
        )
    })?;
    crate::services::master_seed::ensure_master(pool, code, &config.master_user_label).await?;
    Ok(())
}

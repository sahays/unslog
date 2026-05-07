use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

use unslog::config::{AppConfig, LogFormat};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let config = AppConfig::from_env()?;
    let _guards = init_tracing(&config)?;

    tracing::info!(
        format = ?config.log_format,
        log_dir = %config.log_path().display(),
        "tracing initialized",
    );

    unslog::startup::run(config).await
}

/// Compose the tracing subscriber:
///   • stdout writer (compact for humans, json for log shippers)
///   • daily-rotating file writer in `<data_dir>/<log_dir>/unslog-YYYY-MM-DD.log`
///   • EnvFilter (`RUST_LOG`), defaulting to `info,unslog=debug` in dev
///
/// The two `WorkerGuard`s must outlive the program; we return them so `main`
/// can hold them.
fn init_tracing(config: &AppConfig) -> anyhow::Result<Vec<WorkerGuard>> {
    let log_dir = config.log_path();
    std::fs::create_dir_all(&log_dir)?;

    let file_appender = tracing_appender::rolling::daily(&log_dir, "unslog.log");
    let (file_writer, file_guard) = tracing_appender::non_blocking(file_appender);
    let (stdout_writer, stdout_guard) = tracing_appender::non_blocking(std::io::stdout());

    let writer = stdout_writer.and(file_writer);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,unslog=debug,tower_http=info"));

    match config.log_format {
        LogFormat::Json => {
            let layer = tracing_subscriber::fmt::layer()
                .with_writer(writer)
                .json()
                .with_current_span(true)
                .with_span_list(false);
            tracing_subscriber::registry()
                .with(filter)
                .with(layer)
                .init();
        }
        LogFormat::Compact => {
            let layer = tracing_subscriber::fmt::layer()
                .with_writer(writer)
                .with_target(false)
                .compact();
            tracing_subscriber::registry()
                .with(filter)
                .with(layer)
                .init();
        }
    }

    Ok(vec![stdout_guard, file_guard])
}

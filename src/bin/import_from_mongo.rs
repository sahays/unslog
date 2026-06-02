//! `cargo run --bin import_from_mongo` — one-shot Mongo → Postgres copy.
//!
//! Reads every Mongo collection, mints new prefixed Postgres ids, rewrites
//! every FK through the in-memory map, and inserts into the Postgres
//! schema in dependency order. Run once locally after Postgres is up; the
//! bin refuses to write into a populated target unless `--force` is passed.

use anyhow::{Context, Result};
use clap::Parser;
use sqlx::postgres::PgPoolOptions;
use std::env;

use unslog::services::db::url_without_password;
use unslog::services::mongo_import::{self, ImportOptions};

#[derive(Parser, Debug)]
#[command(
    version,
    about = "unslog one-shot Mongo → Postgres importer (Phase A, Step 3)"
)]
struct Cli {
    /// Skip the "target tables must be empty" guard. Use only when
    /// resuming after a partial import — `ON CONFLICT (id) DO NOTHING`
    /// keeps the write pass idempotent at the row level.
    #[arg(long, default_value_t = false)]
    force: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();
    let cli = Cli::parse();

    let mongo_uri = env::var("MONGO_URI").context("MONGO_URI not set in env")?;
    let mongo_db = env::var("MONGO_DB").context("MONGO_DB not set in env")?;
    let database_url = env::var("DATABASE_URL").context("DATABASE_URL not set in env")?;

    tracing::info!(
        event = "import.start",
        mongo_db = %mongo_db,
        postgres_url = %url_without_password(&database_url),
        force = cli.force,
        "starting mongo → postgres import"
    );

    let mongo_client = mongodb::Client::with_uri_str(&mongo_uri)
        .await
        .context("mongo connect")?;
    let mongo_db_handle = mongo_client.database(&mongo_db);

    let pg = PgPoolOptions::new()
        .max_connections(8)
        .connect(&database_url)
        .await
        .context("postgres connect")?;

    let set = mongo_import::run(&mongo_db_handle, &pg, ImportOptions { force: cli.force }).await?;

    log_summary(&set);
    Ok(())
}

fn log_summary(set: &unslog::services::mongo_import::read::ReadSet) {
    tracing::info!(
        event = "import.summary",
        category_count = set.categories.len(),
        company_count = set.companies.len(),
        question_count = set.questions.len(),
        session_count = set.sessions.len(),
        evaluation_count = set.evaluations.len(),
        summary_count = set.summaries.len(),
        story_count = set.stories.len(),
        story_version_count = set.story_versions.len(),
        pitch_count = set.pitches.len(),
        pitch_version_count = set.pitch_versions.len(),
        journal_entry_count = set.journal_entries.len(),
        asset_count = set.assets.len(),
        prompt_count = set.prompts.len(),
        prompt_version_count = set.prompt_versions.len(),
        settings_present = set.settings.is_some(),
        "import complete"
    );
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

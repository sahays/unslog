//! `cargo run --bin eval -- <subcommand>` — local-only quality + regression
//! tooling for unslog's LLM-driven features.
//!
//! Subcommands:
//!
//! * `extract` — Mongo → gold JSON files under data/evals/gold/
//! * `score` — rubric + LLM-judge → Markdown report
//! * `regression` — replay a baseline vs candidate prompt version on the
//!   gold set, diff outputs, optionally judge winners
//!
//! See `src/evals/mod.rs` for architecture. All commands run locally, read
//! the same env (`MONGO_URI`, `MONGO_DB`, `DATA_DIR`, `OPENROUTER_API_KEY`)
//! as the main app, and write reports under `<data_dir>/evals/reports/`.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::sync::Arc;

use unslog::config::AppConfig;
use unslog::db;
use unslog::evals;
use unslog::evals::regression::{self, RegressionOptions};
use unslog::evals::report;
use unslog::evals::score::{self, ScoreOptions};
use unslog::evals::Target;
use unslog::services::db as pgdb;
use unslog::services::openrouter::{LlmClient, OpenRouter};

#[derive(Parser, Debug)]
#[command(
    version,
    about = "unslog eval harness — gold extract / score / regression"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Pull completed stories and packet-bearing companies from Mongo into
    /// data/evals/gold/. Idempotent — re-running overwrites the same files.
    Extract,

    /// Run rubric (always) and optionally the LLM-judge on each gold entry,
    /// writing a Markdown report under data/evals/reports/<timestamp>/.
    Score {
        /// Eval target to score. Defaults to all three.
        #[arg(long, value_parser = parse_target)]
        target: Option<Target>,

        /// Skip the LLM judge — rubric only. Default is to run the judge
        /// when `OPENROUTER_API_KEY` is set; this flag forces rubric-only.
        #[arg(long, default_value_t = false)]
        no_judge: bool,
    },

    /// Replay a named prompt's baseline vs candidate version on the gold set.
    Regression {
        /// Prompt name (e.g. `story_summarize`). See
        /// `evals::regression::SUPPORTED_PROMPTS` for the current list.
        #[arg(long)]
        prompt: String,

        /// Baseline prompt_versions._id (the "known good" version).
        #[arg(long)]
        baseline: String,

        /// Candidate prompt_versions._id (the new version under test).
        #[arg(long)]
        candidate: String,

        /// Also run the LLM judge against the candidate output per entry.
        #[arg(long, default_value_t = false)]
        judge: bool,
    },
}

fn parse_target(s: &str) -> Result<Target, String> {
    Target::parse(s).ok_or_else(|| {
        format!("unknown target {s:?} — expected one of: story_summary, story_chat, company")
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();
    let cli = Cli::parse();
    let config = AppConfig::from_env().context("load env")?;
    let database = db::connect(&config.mongo_uri, &config.mongo_db).await?;
    // Postgres now backs the ported bootstrap-class stores (settings,
    // prompts, categories); the eval harness needs both connections.
    let pool = pgdb::connect_postgres(&config.database_url)
        .await
        .map_err(|e| anyhow::anyhow!("postgres: {e}"))?;

    match cli.cmd {
        Cmd::Extract => {
            let rep = evals::extract::extract_all(&database, &pool, &config.data_dir).await?;
            println!(
                "Extracted {} stories ({} skipped no-version), {} companies → {}",
                rep.stories_written,
                rep.stories_skipped_no_version,
                rep.companies_written,
                evals::gold::gold_dir(&config.data_dir).display()
            );
            Ok(())
        }
        Cmd::Score { target, no_judge } => {
            let targets: Vec<Target> = match target {
                Some(t) => vec![t],
                None => Target::ALL.to_vec(),
            };
            let client = build_client(&config);
            let run_judge = !no_judge && client.is_some();
            if !no_judge && client.is_none() {
                eprintln!(
                    "warning: OPENROUTER_API_KEY not set — running rubric-only (no LLM judge)"
                );
            }
            let opts = ScoreOptions { run_judge };
            let client_ref: Option<&dyn LlmClient> =
                client.as_ref().map(|c| c.as_ref() as &dyn LlmClient);
            let reports = score::score_all(
                &database,
                &pool,
                &config.data_dir,
                &targets,
                client_ref,
                opts,
            )
            .await?;
            let dir = report::open_report_dir(&config.data_dir)?;
            for r in &reports {
                report::write_target_report(&dir, r)?;
            }
            let summary_path = report::write_summary(&dir, &reports)?;
            print_score_summary(&reports);
            println!("Full report: {}", summary_path.display());
            Ok(())
        }
        Cmd::Regression {
            prompt,
            baseline,
            candidate,
            judge,
        } => {
            let client = build_client(&config).ok_or_else(|| {
                anyhow::anyhow!("OPENROUTER_API_KEY required for regression (needs LLM calls)")
            })?;
            let opts = RegressionOptions {
                prompt_name: prompt,
                baseline_version_id: baseline,
                candidate_version_id: candidate,
                run_judge: judge,
            };
            let rep = regression::run(&pool, client.as_ref(), &config.data_dir, opts).await?;
            let dir = report::open_report_dir(&config.data_dir)?;
            let path = regression::write_report(&dir, &rep)?;
            println!(
                "Regression run on {} entries → {}",
                rep.entries.len(),
                path.display()
            );
            Ok(())
        }
    }
}

fn build_client(config: &AppConfig) -> Option<Arc<OpenRouter>> {
    if !config.openrouter_configured() {
        return None;
    }
    Some(Arc::new(OpenRouter::new(
        reqwest::Client::new(),
        config.openrouter_api_key.clone(),
        config.referer.clone(),
    )))
}

fn print_score_summary(reports: &[report::TargetReport]) {
    println!();
    for r in reports {
        let n = r.entries.len();
        let passed = r.entries.iter().filter(|e| e.rubric.passed()).count();
        let judge_summary = r
            .entries
            .iter()
            .filter_map(|e| e.judge.as_ref().and_then(|j| j.aggregate))
            .collect::<Vec<_>>();
        let judge_str = if judge_summary.is_empty() {
            String::from("judge: skipped")
        } else {
            let avg = judge_summary.iter().sum::<f32>() / judge_summary.len() as f32;
            format!(
                "judge avg {avg:.2}/5 across {} entries",
                judge_summary.len()
            )
        };
        println!(
            "  {:>14} — rubric {passed}/{n}, {judge_str}",
            r.target.as_str()
        );
    }
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

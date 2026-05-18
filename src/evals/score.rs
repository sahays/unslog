//! Score orchestrator — load gold from disk, run rubric checks for each
//! entry, optionally run the LLM-judge against itself, package into
//! [`TargetReport`]s ready for the report writer.
//!
//! "Judge against itself" means: with no fresh candidate generation in the
//! pipeline (that's regression's job), the judge scores the gold entry
//! *as the candidate* with the same gold entry as the reference. The signal
//! is "is this gold entry actually 5/5?" — anything < 5 across the
//! dimensions surfaces a weak reference the user should consider pruning.

use anyhow::Result;
use mongodb::Database;

use crate::evals::adversarial;
use crate::evals::gold;
use crate::evals::judge::{self, JudgeResult};
use crate::evals::report::{EntryReport, TargetReport};
use crate::evals::rubric;
use crate::evals::Target;
use crate::services::openrouter::LlmClient;

#[derive(Debug, Clone, Copy)]
pub struct ScoreOptions {
    pub run_judge: bool,
}

pub async fn score_all(
    db: &Database,
    data_dir: &str,
    targets: &[Target],
    client: Option<&dyn LlmClient>,
    opts: ScoreOptions,
) -> Result<Vec<TargetReport>> {
    let mut reports = Vec::new();
    for &target in targets {
        let report = match target {
            Target::StorySummary => score_story_summary(data_dir, client, opts).await?,
            Target::StoryChat => score_story_chat(data_dir, client, opts).await?,
            Target::Company => score_company(data_dir, client, opts).await?,
            Target::Adversarial => adversarial::score_adversarial(db, client, data_dir).await?,
        };
        reports.push(report);
    }
    Ok(reports)
}

async fn score_story_summary(
    data_dir: &str,
    client: Option<&dyn LlmClient>,
    opts: ScoreOptions,
) -> Result<TargetReport> {
    let stories = gold::load_stories(data_dir)?;
    let mut entries = Vec::with_capacity(stories.len());
    for story in &stories {
        let rubric = rubric::check_story_summary(story);
        let judge = run_judge_if_enabled(opts, client, || async {
            judge::judge_story_summary_self(client.unwrap(), story).await
        })
        .await;
        entries.push(EntryReport { rubric, judge });
    }
    Ok(TargetReport {
        target: Target::StorySummary,
        entries,
    })
}

async fn score_story_chat(
    data_dir: &str,
    client: Option<&dyn LlmClient>,
    opts: ScoreOptions,
) -> Result<TargetReport> {
    let stories = gold::load_stories(data_dir)?;
    let mut entries = Vec::with_capacity(stories.len());
    for story in &stories {
        let rubric = rubric::check_story_chat(story);
        let judge = run_judge_if_enabled(opts, client, || async {
            judge::judge_story_chat(client.unwrap(), story).await
        })
        .await;
        entries.push(EntryReport { rubric, judge });
    }
    Ok(TargetReport {
        target: Target::StoryChat,
        entries,
    })
}

async fn score_company(
    data_dir: &str,
    client: Option<&dyn LlmClient>,
    opts: ScoreOptions,
) -> Result<TargetReport> {
    let companies = gold::load_companies(data_dir)?;
    let mut entries = Vec::with_capacity(companies.len());
    for company in &companies {
        let rubric = rubric::check_company(company);
        let judge = run_judge_if_enabled(opts, client, || async {
            judge::judge_company_self(client.unwrap(), company).await
        })
        .await;
        entries.push(EntryReport { rubric, judge });
    }
    Ok(TargetReport {
        target: Target::Company,
        entries,
    })
}

/// Helper that runs an async judge call only when judging is enabled AND a
/// client is provided. Keeps the per-target functions readable.
async fn run_judge_if_enabled<F, Fut>(
    opts: ScoreOptions,
    client: Option<&dyn LlmClient>,
    f: F,
) -> Option<JudgeResult>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = JudgeResult>,
{
    if opts.run_judge && client.is_some() {
        Some(f().await)
    } else {
        None
    }
}

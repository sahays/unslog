//! Prompt regression replay — given two `prompt_versions` IDs for the same
//! prompt name, regenerate the corresponding output for every gold entry
//! against both versions and diff. Optionally ask the judge which is better.
//!
//! Scope for v1: supports the `story_summarize` prompt only. This is the
//! call that turns a chat transcript into a StoryBody — exactly what gets
//! pinned into a StoryVersion when the user clicks "Generate" in the UI.
//! Other prompts (research, story_chat, critique) need either web-search
//! orchestration or multi-turn replay, neither of which is in this slice.

use anyhow::{Context, Result};
use sqlx::PgPool;

use crate::error::AppError;
use crate::evals::gold;
use crate::evals::judge::{self, JudgeResult, JUDGE_MODEL};
use crate::models::{ChatRole, StoryBody};
use crate::services::openrouter::{self, ChatMessage, LlmClient};
use crate::services::{prompt_store, settings_store};

pub const SUPPORTED_PROMPTS: &[&str] = &["story_summarize"];

pub struct RegressionEntry {
    pub gold_id: String,
    pub label: String,
    pub baseline_body: Option<StoryBody>,
    pub candidate_body: Option<StoryBody>,
    pub baseline_error: Option<String>,
    pub candidate_error: Option<String>,
    pub judge: Option<JudgeResult>,
}

pub struct RegressionReport {
    pub prompt_name: String,
    pub baseline_version_id: String,
    pub candidate_version_id: String,
    pub model: String,
    pub judge_model: String,
    pub entries: Vec<RegressionEntry>,
}

pub struct RegressionOptions {
    pub prompt_name: String,
    pub baseline_version_id: String,
    pub candidate_version_id: String,
    /// Run the LLM judge to compare baseline vs candidate per gold entry on
    /// the story_summary dimensions. Independent of cost — each entry
    /// adds one grok-4.3 call.
    pub run_judge: bool,
}

pub async fn run(
    pool: &PgPool,
    client: &dyn LlmClient,
    data_dir: &str,
    opts: RegressionOptions,
) -> Result<RegressionReport> {
    if !SUPPORTED_PROMPTS.contains(&opts.prompt_name.as_str()) {
        anyhow::bail!(
            "regression for prompt {:?} not supported — supported: {:?}",
            opts.prompt_name,
            SUPPORTED_PROMPTS
        );
    }

    let baseline = load_prompt_body(pool, &opts.baseline_version_id).await?;
    let candidate = load_prompt_body(pool, &opts.candidate_version_id).await?;
    let settings = settings_store::load(pool).await?;
    let model = settings.critique_model.clone();

    let stories = gold::load_stories(data_dir)?;
    let mut entries = Vec::with_capacity(stories.len());
    for story in &stories {
        let user = build_user_message(story);
        let baseline_res = generate_body(client, &model, &baseline, &user).await;
        let candidate_res = generate_body(client, &model, &candidate, &user).await;
        let (baseline_body, baseline_error) = split_result(baseline_res);
        let (candidate_body, candidate_error) = split_result(candidate_res);

        let judge = if opts.run_judge && candidate_body.is_some() {
            let candidate_md = match &candidate_body {
                Some(b) => render_body(b),
                None => String::new(),
            };
            Some(judge::judge_story_summary(client, story, &candidate_md).await)
        } else {
            None
        };

        entries.push(RegressionEntry {
            gold_id: story.id.clone(),
            label: format!("{} — v{}", story.competency_name, story.current_version_n),
            baseline_body,
            candidate_body,
            baseline_error,
            candidate_error,
            judge,
        });
    }

    Ok(RegressionReport {
        prompt_name: opts.prompt_name,
        baseline_version_id: opts.baseline_version_id,
        candidate_version_id: opts.candidate_version_id,
        model,
        judge_model: JUDGE_MODEL.to_string(),
        entries,
    })
}

async fn load_prompt_body(pool: &PgPool, version_id: &str) -> Result<String> {
    let v = prompt_store::get_version(pool, version_id)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .with_context(|| format!("prompt version {version_id} not found"))?;
    Ok(v.body)
}

fn build_user_message(story: &gold::StoryGold) -> String {
    let mut transcript = String::new();
    for turn in &story.chat {
        let who = match turn.role {
            ChatRole::Assistant => "coach",
            ChatRole::User => "candidate",
        };
        transcript.push_str(&format!("[{who}] {}\n", turn.content));
    }
    format!("<chat_transcript>\n{transcript}</chat_transcript>")
}

async fn generate_body(
    client: &dyn LlmClient,
    model: &str,
    prompt_body: &str,
    user: &str,
) -> Result<StoryBody, AppError> {
    let messages = vec![
        ChatMessage::system(prompt_body.to_string()),
        ChatMessage::user(user.to_string()),
    ];
    let raw = client.chat(model, messages, true).await?;
    let body: StoryBody = openrouter::parse_json(&raw)?;
    Ok(body)
}

fn split_result(r: Result<StoryBody, AppError>) -> (Option<StoryBody>, Option<String>) {
    match r {
        Ok(b) => (Some(b), None),
        Err(e) => (None, Some(format!("{e}"))),
    }
}

fn render_body(b: &StoryBody) -> String {
    let mut s = String::new();
    for (label, section) in [
        ("Situation", &b.situation),
        ("Task", &b.task),
        ("Action", &b.action),
        ("Result", &b.result),
        ("Reflection", &b.reflection),
    ] {
        s.push_str(&format!("## {label}\n"));
        for bullet in section {
            s.push_str(&format!("- {bullet}\n"));
        }
        s.push('\n');
    }
    s
}

// ── Report writer ──────────────────────────────────────────────────────

use std::fmt::Write as _;
use std::path::PathBuf;

pub fn write_report(
    dir: &crate::evals::report::ReportDir,
    report: &RegressionReport,
) -> Result<PathBuf> {
    let path = dir
        .path
        .join(format!("regression_{}.md", report.prompt_name));
    let mut s = String::new();
    let _ = writeln!(s, "# Regression — prompt `{}`", report.prompt_name);
    let _ = writeln!(s, "* baseline version: `{}`", report.baseline_version_id);
    let _ = writeln!(s, "* candidate version: `{}`", report.candidate_version_id);
    let _ = writeln!(s, "* generation model: `{}`", report.model);
    let _ = writeln!(s, "* judge model: `{}`", report.judge_model);
    let _ = writeln!(s, "* entries: {}", report.entries.len());
    let _ = writeln!(s);
    for entry in &report.entries {
        let _ = writeln!(s, "## {} — `{}`", entry.label, entry.gold_id);
        if let Some(err) = &entry.baseline_error {
            let _ = writeln!(s, "- ⚠️ baseline generation failed: {err}");
        }
        if let Some(err) = &entry.candidate_error {
            let _ = writeln!(s, "- ⚠️ candidate generation failed: {err}");
        }
        match (&entry.baseline_body, &entry.candidate_body) {
            (Some(b), Some(c)) => {
                let _ = writeln!(s, "### Diff (baseline → candidate)");
                let _ = writeln!(s, "```diff");
                s.push_str(&diff_bodies(b, c));
                let _ = writeln!(s, "```");
            }
            _ => {
                let _ = writeln!(
                    s,
                    "_(skipping diff — one or both bodies failed to generate)_"
                );
            }
        }
        if let Some(j) = &entry.judge {
            if let Some(agg) = j.aggregate {
                let _ = writeln!(s, "- **Judge on candidate:** aggregate {agg:.2}/5");
            }
            for d in &j.dimensions {
                let _ = writeln!(
                    s,
                    "    - **{}**: {}/5 — {}",
                    d.dimension, d.score, d.justification
                );
            }
            if let Some(err) = &j.error {
                let _ = writeln!(s, "    - ⚠️ judge error: {err}");
            }
        }
        let _ = writeln!(s);
    }
    std::fs::write(&path, s).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

fn diff_bodies(b: &StoryBody, c: &StoryBody) -> String {
    let mut s = String::new();
    diff_section(&mut s, "Situation", &b.situation, &c.situation);
    diff_section(&mut s, "Task", &b.task, &c.task);
    diff_section(&mut s, "Action", &b.action, &c.action);
    diff_section(&mut s, "Result", &b.result, &c.result);
    diff_section(&mut s, "Reflection", &b.reflection, &c.reflection);
    s
}

fn diff_section(out: &mut String, label: &str, baseline: &[String], candidate: &[String]) {
    let _ = writeln!(out, "@@ {label} @@");
    let bset: std::collections::HashSet<&str> = baseline.iter().map(String::as_str).collect();
    let cset: std::collections::HashSet<&str> = candidate.iter().map(String::as_str).collect();
    for b in baseline {
        if !cset.contains(b.as_str()) {
            let _ = writeln!(out, "- {b}");
        }
    }
    for c in candidate {
        if !bset.contains(c.as_str()) {
            let _ = writeln!(out, "+ {c}");
        }
    }
    for shared in baseline.iter().filter(|b| cset.contains(b.as_str())) {
        let _ = writeln!(out, "  {shared}");
    }
}

//! On-disk gold set — types + filesystem layout + load/save.
//!
//! The gold set lives at `<data_dir>/evals/gold/{stories,companies}/<id>.json`
//! and IS the contract the eval suite grades against. It's committed to the
//! repo (versioned alongside the prompts it validates). After `extract`, the
//! user is expected to skim and delete any entry they wouldn't ship as a
//! "good" reference.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::models::{ChatRole, Difficulty, ResearchPacket, StoryBody};

const GOLD_SUBDIR: &str = "evals/gold";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryGold {
    pub id: String,
    pub competency_id: String,
    pub competency_name: String,
    pub mode: Difficulty,
    pub chat: Vec<ChatTurnGold>,
    pub current_version_n: u32,
    pub body: StoryBody,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTurnGold {
    pub role: ChatRole,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyGold {
    pub id: String,
    pub name: String,
    pub role: String,
    pub canonical_role: String,
    pub packet: ResearchPacket,
}

/// Hand-written adversarial test case. Unlike `StoryGold` / `CompanyGold`
/// these are not extracted from Mongo — they're curated examples of
/// prompt-injection attempts the suite checks the model's resistance
/// against. Each case targets one feature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdversarialGold {
    pub id: String,
    pub description: String,
    /// Which feature pipeline this case targets. Currently only
    /// `"story_summarize"` is wired into the runner.
    pub feature: String,
    /// Display name used in the eval report.
    pub competency_name: String,
    /// Chat turns to feed into the pipeline. The injection lives in one or
    /// more of the `user` turns. Mode is irrelevant for `story_summarize`
    /// but kept for symmetry with `StoryGold`.
    pub chat: Vec<ChatTurnGold>,
    /// Substrings the suite searches for in the model's output. If any
    /// appears (case-insensitive substring), the model failed to resist.
    pub resistance_markers: Vec<String>,
}

/// Root of the gold set, computed from the same `DATA_DIR` env that the app
/// uses (defaulting to `./data`). Keeping it under data/ means it's visible
/// next to logs and recordings rather than buried in src/.
pub fn gold_dir(data_dir: &str) -> PathBuf {
    PathBuf::from(data_dir).join(GOLD_SUBDIR)
}

pub fn stories_dir(data_dir: &str) -> PathBuf {
    gold_dir(data_dir).join("stories")
}

pub fn companies_dir(data_dir: &str) -> PathBuf {
    gold_dir(data_dir).join("companies")
}

pub fn adversarial_dir(data_dir: &str) -> PathBuf {
    gold_dir(data_dir).join("adversarial")
}

pub fn save_story(data_dir: &str, gold: &StoryGold) -> Result<PathBuf> {
    let dir = stories_dir(data_dir);
    std::fs::create_dir_all(&dir).with_context(|| format!("mkdir -p {}", dir.display()))?;
    let path = dir.join(format!("{}.json", gold.id));
    write_json(&path, gold)?;
    Ok(path)
}

pub fn save_company(data_dir: &str, gold: &CompanyGold) -> Result<PathBuf> {
    let dir = companies_dir(data_dir);
    std::fs::create_dir_all(&dir).with_context(|| format!("mkdir -p {}", dir.display()))?;
    let path = dir.join(format!("{}.json", gold.id));
    write_json(&path, gold)?;
    Ok(path)
}

pub fn load_stories(data_dir: &str) -> Result<Vec<StoryGold>> {
    load_dir(&stories_dir(data_dir))
}

pub fn load_companies(data_dir: &str) -> Result<Vec<CompanyGold>> {
    load_dir(&companies_dir(data_dir))
}

pub fn load_adversarial(data_dir: &str) -> Result<Vec<AdversarialGold>> {
    load_dir(&adversarial_dir(data_dir))
}

fn load_dir<T: for<'de> Deserialize<'de>>(dir: &Path) -> Result<Vec<T>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("read_dir {}", dir.display()))?
        .filter_map(Result::ok)
        .collect();
    // Sort for stable report ordering.
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let bytes = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        let parsed: T =
            serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
        out.push(parsed);
    }
    Ok(out)
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    std::fs::write(path, bytes).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

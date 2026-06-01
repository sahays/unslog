//! LLM prompt assembly for the curator. Pure functions over the curator's
//! candidate pool + canonical category list + recent weaknesses + recently-
//! asked exclusion set.

use std::collections::HashSet;

use serde::Deserialize;

use crate::error::AppError;
use crate::models::{Category, Summary};
use crate::services::openrouter::{parse_json, ChatMessage, LlmClient};

use super::PoolItem;

pub(super) const TARGET_MIN: usize = 4;
pub(super) const TARGET_MAX: usize = 6;
const TARGET_DEFAULT: usize = 5;

#[derive(Debug, Deserialize)]
pub(super) struct CuratorJson {
    #[serde(default)]
    pub question_ids: Vec<String>,
    #[serde(default)]
    pub focus_line: String,
}

pub(super) async fn try_llm_curate(
    or: &dyn LlmClient,
    lite_model: &str,
    pool: &[PoolItem],
    recent_summaries: &[Summary],
    recently_asked: &HashSet<String>,
    canonical: &[Category],
) -> Result<CuratorJson, AppError> {
    let user = render_user(pool, recent_summaries, recently_asked, canonical);
    let system = "You curate a focused behavioral-interview practice session. From a candidate question pool, pick a tight ordered set the user will actually benefit from: bias toward categories where past sessions showed recurring weaknesses, avoid recently-asked questions if alternatives exist, diversify categories so the same competency isn't tested twice in a row, and order the picks to build narrative coherence (warm up, escalate, reflective close).";

    let raw = or
        .chat(
            lite_model,
            vec![ChatMessage::system(system), ChatMessage::user(user)],
            true,
        )
        .await?;
    parse_json(&raw)
}

fn render_user(
    pool: &[PoolItem],
    recent_summaries: &[Summary],
    recently_asked: &HashSet<String>,
    canonical: &[Category],
) -> String {
    let canonical_block = render_canonical(canonical);
    let pool_block = render_pool(pool, recently_asked);
    let weaknesses_block = render_weaknesses(recent_summaries);
    let target_n = TARGET_DEFAULT;

    format!(
        r#"<canonical_categories>
{canonical_block}
</canonical_categories>

<pool>
{pool_block}
</pool>

<recent_weaknesses>
{weaknesses_block}
</recent_weaknesses>

Pick {TARGET_MIN}–{TARGET_MAX} questions (target {target_n}) from the pool, in answer order. Return ONLY a JSON object with this shape:

{{
  "question_ids": ["<id-from-pool>", "<id-from-pool>", ...],
  "focus_line": "<one short sentence describing today's focus, e.g. 'Today's focus is ambiguity, where you stumbled last session, plus a fresh ownership story for breadth.'>"
}}

Rules:
- Use ONLY question IDs that appear in the pool above. No fabricated IDs.
- {TARGET_MIN} ≤ count ≤ {TARGET_MAX}.
- Never repeat an ID.
- If the pool is small (< {TARGET_MIN}), include every available question."#
    )
}

fn render_canonical(canonical: &[Category]) -> String {
    canonical
        .iter()
        .map(|c| format!("- `{}`: {}", c.id, c.name))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_pool(pool: &[PoolItem], recently_asked: &HashSet<String>) -> String {
    pool.iter()
        .map(|p| render_pool_item(p, recently_asked))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_pool_item(p: &PoolItem, recently_asked: &HashSet<String>) -> String {
    let cats = if p.categories.is_empty() {
        "(untagged)".to_string()
    } else {
        p.categories.join(", ")
    };
    let recent_marker = if recently_asked.contains(&p.id) {
        " (RECENTLY ASKED — avoid unless pool is too small)"
    } else {
        ""
    };
    format!(
        "- {} | categories: [{cats}] | text: {}{recent_marker}",
        p.id, p.text
    )
}

fn render_weaknesses(recent_summaries: &[Summary]) -> String {
    if recent_summaries.is_empty() {
        return "(no prior session summaries — no weakness bias)".to_string();
    }
    recent_summaries
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let weak = if s.recurring_weaknesses.is_empty() {
                "(none recorded)".to_string()
            } else {
                s.recurring_weaknesses.join("; ")
            };
            format!("Session -{} weaknesses: {}", i + 1, weak)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

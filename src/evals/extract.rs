//! Gold-set extraction — Postgres → JSON files under `data/evals/gold/`.
//!
//! Pulls:
//!   * Every `Story` with `status = "complete"`, joined with its current
//!     `StoryVersion` (the user-accepted body) and its competency's display
//!     name. Stories without a `current_version_id` are skipped — they don't
//!     have a body to grade against.
//!   * Every `Company` with `research_packet != null`.
//!
//! The extractor is idempotent: re-running overwrites the same files.
//! "Accepted ≠ ideal gold," so the README in `data/evals/gold/` reminds the
//! user to prune entries they wouldn't ship as references.

use std::collections::HashMap;

use anyhow::{Context, Result};
use sqlx::PgPool;

use crate::evals::gold::{self, ChatTurnGold, CompanyGold, StoryGold};
use crate::models::{Category, StoryStatus};
use crate::services::master_seed::MASTER_ID;
use crate::services::{category_store, company_store, story_store, story_version_store};

pub struct ExtractReport {
    pub stories_written: usize,
    pub stories_skipped_no_version: usize,
    pub companies_written: usize,
}

pub async fn extract_all(pool: &PgPool, data_dir: &str) -> Result<ExtractReport> {
    let stories = extract_stories(pool, data_dir).await?;
    let companies_written = extract_companies(pool, data_dir).await?;
    Ok(ExtractReport {
        stories_written: stories.0,
        stories_skipped_no_version: stories.1,
        companies_written,
    })
}

async fn extract_stories(pool: &PgPool, data_dir: &str) -> Result<(usize, usize)> {
    let stories = story_store::list_completed(pool, MASTER_ID)
        .await
        .context("query completed stories")?;

    let version_ids: Vec<String> = stories
        .iter()
        .filter_map(|s| s.current_version_id.clone())
        .collect();
    let versions_by_id = story_version_store::list_by_ids(pool, MASTER_ID, &version_ids)
        .await
        .context("query story versions")?;

    let competencies_by_id = load_competencies_by_id(pool).await?;

    let mut written = 0;
    let mut skipped = 0;
    for story in stories {
        // Defensive: list_completed already filters by status = complete,
        // but double-check in case the row was racy at read time.
        if !matches!(story.status, StoryStatus::Complete) {
            continue;
        }
        let Some(vid) = story.current_version_id.as_deref() else {
            skipped += 1;
            continue;
        };
        let Some(version) = versions_by_id.get(vid) else {
            skipped += 1;
            continue;
        };
        let competency_name = competencies_by_id
            .get(&story.competency_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "(unknown competency)".into());
        let gold = StoryGold {
            id: story.id.clone(),
            competency_id: story.competency_id.clone(),
            competency_name,
            mode: story.mode,
            chat: story
                .chat
                .iter()
                .map(|t| ChatTurnGold {
                    role: t.role,
                    content: t.content.clone(),
                })
                .collect(),
            current_version_n: version.version_n,
            body: version.body.clone(),
        };
        gold::save_story(data_dir, &gold)?;
        written += 1;
    }
    Ok((written, skipped))
}

/// Snapshot the full category list keyed by id so each story can look up
/// its competency display name without an N+1 fan-out.
async fn load_competencies_by_id(pool: &PgPool) -> Result<HashMap<String, Category>> {
    let all = category_store::list_all(pool)
        .await
        .context("query competencies")?;
    Ok(all.into_iter().map(|c| (c.id.clone(), c)).collect())
}

/// Pull every owner-scoped company with a research packet from Postgres.
/// `list_by_name` returns the full row including the packet; filter
/// client-side since the eval bin has no need for a partial-index query.
async fn extract_companies(pool: &PgPool, data_dir: &str) -> Result<usize> {
    let companies = company_store::list_by_name(pool, MASTER_ID)
        .await
        .context("query companies with packets")?;
    let mut written = 0;
    for company in companies {
        let Some(packet) = company.research_packet else {
            continue;
        };
        let gold = CompanyGold {
            id: company.id,
            name: company.name,
            role: company.role,
            canonical_role: company.canonical_role.as_str().to_string(),
            packet,
        };
        gold::save_company(data_dir, &gold)?;
        written += 1;
    }
    Ok(written)
}

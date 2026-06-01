//! Gold-set extraction — Mongo → JSON files under `data/evals/gold/`.
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
use futures::TryStreamExt;
use mongodb::Database;
use sqlx::PgPool;

use crate::evals::gold::{self, ChatTurnGold, CompanyGold, StoryGold};
use crate::models::{Category, Story, StoryStatus, StoryVersion};
use crate::services::company_store;

pub struct ExtractReport {
    pub stories_written: usize,
    pub stories_skipped_no_version: usize,
    pub companies_written: usize,
}

pub async fn extract_all(db: &Database, pool: &PgPool, data_dir: &str) -> Result<ExtractReport> {
    let stories = extract_stories(db, data_dir).await?;
    let companies_written = extract_companies(pool, data_dir).await?;
    Ok(ExtractReport {
        stories_written: stories.0,
        stories_skipped_no_version: stories.1,
        companies_written,
    })
}

async fn extract_stories(db: &Database, data_dir: &str) -> Result<(usize, usize)> {
    let stories: Vec<Story> = db
        .collection::<Story>(Story::COLLECTION)
        .find(bson::doc! { "status": "complete" })
        .await?
        .try_collect()
        .await
        .context("query completed stories")?;

    // Bulk-load the current StoryVersion docs and the competency names so we
    // don't fan out to N+1 lookups.
    let version_ids: Vec<&str> = stories
        .iter()
        .filter_map(|s| s.current_version_id.as_deref())
        .collect();
    let versions_by_id = if version_ids.is_empty() {
        HashMap::new()
    } else {
        let raw: Vec<StoryVersion> = db
            .collection::<StoryVersion>(StoryVersion::COLLECTION)
            .find(bson::doc! { "_id": { "$in": &version_ids } })
            .await?
            .try_collect()
            .await
            .context("query story versions")?;
        raw.into_iter().map(|v| (v.id.clone(), v)).collect()
    };

    let competency_ids: Vec<&str> = stories.iter().map(|s| s.competency_id.as_str()).collect();
    let competencies_by_id = if competency_ids.is_empty() {
        HashMap::new()
    } else {
        let raw: Vec<Category> = db
            .collection::<Category>(Category::COLLECTION)
            .find(bson::doc! { "_id": { "$in": &competency_ids } })
            .await?
            .try_collect()
            .await
            .context("query competencies")?;
        raw.into_iter().map(|c| (c.id.clone(), c)).collect()
    };

    let mut written = 0;
    let mut skipped = 0;
    for story in stories {
        // status == Complete is the Mongo filter; double-check the enum in
        // case old data has a mismatched encoding.
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

/// Pull every owner-scoped company with a research packet from Postgres.
/// `list_by_name` returns the full row including the packet; filter
/// client-side since the eval bin has no need for a partial-index query.
async fn extract_companies(pool: &PgPool, data_dir: &str) -> Result<usize> {
    let companies = company_store::list_by_name(pool)
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

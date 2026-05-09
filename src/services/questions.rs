//! Top-level Question CRUD. Replaces the old per-company embedded-array
//! `QuestionBank.questions[]`. Each question is its own document.

use std::collections::HashSet;

use futures::TryStreamExt;
use mongodb::options::FindOptions;
use mongodb::{Collection, Database};
use rand::seq::SliceRandom;

use crate::error::AppError;
use crate::models::{Question, QuestionSource, Role};
use crate::services::categorize;
use crate::services::category_store;
use crate::services::openrouter::LlmClient;
use crate::services::settings_store;

fn coll(db: &Database) -> Collection<Question> {
    db.collection(Question::COLLECTION)
}

/// All questions tagged for a company, sorted oldest-first (so the agent's
/// initial picks stay at the top).
pub async fn list_for_company(db: &Database, company_id: &str) -> Result<Vec<Question>, AppError> {
    let opts = FindOptions::builder()
        .sort(bson::doc! { "added_at": 1 })
        .build();
    let cursor = coll(db)
        .find(bson::doc! { "company_id": company_id })
        .with_options(opts)
        .await?;
    Ok(cursor.try_collect().await?)
}

/// All questions for a role across the given company set + role-only
/// questions (`company_id = null`). Used by the curator.
///
/// If `company_ids` is empty, returns only role-only questions for `role`.
pub async fn list_for_pool(
    db: &Database,
    role: Role,
    company_ids: &[String],
) -> Result<Vec<Question>, AppError> {
    let mut or_clauses = vec![bson::doc! { "company_id": { "$eq": null } }];
    if !company_ids.is_empty() {
        or_clauses.push(bson::doc! { "company_id": { "$in": company_ids } });
    }
    let filter = bson::doc! {
        "role": role.as_str(),
        "$or": or_clauses,
    };
    let cursor = coll(db).find(filter).await?;
    Ok(cursor.try_collect().await?)
}

pub async fn get(db: &Database, id: &str) -> Result<Option<Question>, AppError> {
    Ok(coll(db).find_one(bson::doc! { "_id": id }).await?)
}

/// Create one or more questions in bulk.
///
/// `categories_for(text)` returns the category tags to apply to each input
/// — used so the caller can do per-text classification (via `services::categorize`)
/// before insertion. Pass a closure returning `Vec::new()` for untagged.
pub async fn append<F>(
    db: &Database,
    texts: impl IntoIterator<Item = String>,
    source: QuestionSource,
    role: Role,
    company_id: Option<String>,
    mut categories_for: F,
) -> Result<usize, AppError>
where
    F: FnMut(&str) -> Vec<String>,
{
    let new_questions: Vec<Question> = texts
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .map(|t| {
            let cats = categories_for(&t);
            Question::new(t, source, role, cats, company_id.clone())
        })
        .collect();
    if new_questions.is_empty() {
        return Ok(0);
    }
    let count = new_questions.len();
    coll(db).insert_many(&new_questions).await?;
    Ok(count)
}

pub async fn update(
    db: &Database,
    id: &str,
    text: &str,
    role: Role,
    categories: &[String],
) -> Result<(), AppError> {
    coll(db)
        .update_one(
            bson::doc! { "_id": id },
            bson::doc! { "$set": {
                "text": text,
                "role": role.as_str(),
                "categories": categories,
            } },
        )
        .await?;
    Ok(())
}

pub async fn delete(db: &Database, id: &str) -> Result<(), AppError> {
    coll(db).delete_one(bson::doc! { "_id": id }).await?;
    Ok(())
}

/// When a company is deleted, decide what to do with its questions. We
/// **delete** them (cascade) — they were created in that company's context
/// and don't transfer cleanly. Role-only questions (company_id is null) are
/// untouched.
pub async fn delete_for_company(db: &Database, company_id: &str) -> Result<u64, AppError> {
    let res = coll(db)
        .delete_many(bson::doc! { "company_id": company_id })
        .await?;
    Ok(res.deleted_count)
}

/// Pick one random unanswered question from a list. Used as the curator
/// fallback path inside `next_question`.
pub fn pick_random<'a>(pool: &'a [Question], seen: &HashSet<String>) -> Option<&'a Question> {
    let candidates: Vec<&Question> = pool.iter().filter(|q| !seen.contains(&q.id)).collect();
    candidates.choose(&mut rand::thread_rng()).copied()
}

/// Tag a batch of question texts via the lite_model classifier and persist
/// them. Three callers (manual paste, agent-suggested, packet-refresh) share
/// this exact "load settings → load canonical → classify → zip → append"
/// shape, so it lives here. Returns the number of questions inserted.
pub async fn categorize_and_append(
    db: &Database,
    openrouter: &dyn LlmClient,
    texts: Vec<String>,
    source: QuestionSource,
    role: Role,
    company_id: Option<String>,
) -> Result<usize, AppError> {
    if texts.is_empty() {
        return Ok(0);
    }
    let settings = settings_store::load(db).await?;
    let canonical = category_store::list_all(db).await?;
    let tags =
        categorize::classify_batch(openrouter, &settings.lite_model, &texts, &canonical).await;
    let by_text: std::collections::HashMap<String, Vec<String>> =
        texts.iter().cloned().zip(tags.into_iter()).collect();
    append(db, texts, source, role, company_id, |t| {
        by_text.get(t).cloned().unwrap_or_default()
    })
    .await
}

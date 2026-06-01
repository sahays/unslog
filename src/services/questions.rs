//! Top-level Question CRUD on Postgres. Each question is its own row in
//! the `questions` table; replaces the old per-company embedded array.
//!
//! Every read/write is owner-scoped via `CurrentUser::id` threaded through
//! by the handler layer. `categories` is JSONB on the column side; `source`
//! and `role` are TEXT + CHECK and round-trip through `as_str` / `parse`.

use std::collections::HashSet;

use rand::seq::SliceRandom;
use sqlx::types::Json;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{Company, Question, QuestionSource, Role};
use crate::services::categorize;
use crate::services::category_store;
use crate::services::openrouter::LlmClient;
use crate::services::settings_store;

#[path = "questions_row.rs"]
mod row;
use row::{source_str, QuestionRow, QUESTION_COLS};

// ── Reads ────────────────────────────────────────────────────────────────

/// All questions tagged for a company, sorted oldest-first.
pub async fn list_for_company(
    pool: &PgPool,
    owner_id: &str,
    company_id: &str,
) -> Result<Vec<Question>, AppError> {
    let sql = format!(
        "SELECT {QUESTION_COLS} FROM questions
         WHERE owner_id = $1 AND company_id = $2
         ORDER BY added_at ASC"
    );
    let rows: Vec<QuestionRow> = sqlx::query_as(&sql)
        .bind(owner_id)
        .bind(company_id)
        .fetch_all(pool)
        .await?;
    rows.into_iter()
        .map(QuestionRow::try_into_question)
        .collect()
}

/// All questions for a role across the given company set + role-only
/// questions (`company_id IS NULL`). Used by the curator.
pub async fn list_for_pool(
    pool: &PgPool,
    owner_id: &str,
    role: Role,
    company_ids: &[String],
) -> Result<Vec<Question>, AppError> {
    let sql = format!(
        "SELECT {QUESTION_COLS} FROM questions
         WHERE owner_id = $1
           AND role = $2
           AND (company_id IS NULL OR company_id = ANY($3))"
    );
    let rows: Vec<QuestionRow> = sqlx::query_as(&sql)
        .bind(owner_id)
        .bind(role.as_str())
        .bind(company_ids)
        .fetch_all(pool)
        .await?;
    rows.into_iter()
        .map(QuestionRow::try_into_question)
        .collect()
}

pub async fn get(pool: &PgPool, owner_id: &str, id: &str) -> Result<Option<Question>, AppError> {
    let sql = format!(
        "SELECT {QUESTION_COLS} FROM questions
         WHERE owner_id = $1 AND id = $2"
    );
    let row: Option<QuestionRow> = sqlx::query_as(&sql)
        .bind(owner_id)
        .bind(id)
        .fetch_optional(pool)
        .await?;
    row.map(QuestionRow::try_into_question).transpose()
}

// ── Writes ───────────────────────────────────────────────────────────────

/// Insert one or more questions in a single multi-row INSERT. `categories_for`
/// returns the category tag list for each input text (callers run the lite
/// classifier upstream and capture it in a closure).
pub async fn append<F>(
    pool: &PgPool,
    owner_id: &str,
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
            Question::new(
                owner_id.to_string(),
                t,
                source,
                role,
                cats,
                company_id.clone(),
            )
        })
        .collect();
    if new_questions.is_empty() {
        return Ok(0);
    }
    let count = new_questions.len();
    insert_many(pool, &new_questions).await?;
    Ok(count)
}

/// Bulk insert via `UNNEST` so N questions ship in one round-trip.
async fn insert_many(pool: &PgPool, rows: &[Question]) -> Result<(), AppError> {
    let ids: Vec<&str> = rows.iter().map(|q| q.id.as_str()).collect();
    let owners: Vec<&str> = rows.iter().map(|q| q.owner_id.as_str()).collect();
    let texts: Vec<&str> = rows.iter().map(|q| q.text.as_str()).collect();
    let sources: Vec<&str> = rows.iter().map(|q| source_str(q.source)).collect();
    let roles: Vec<&str> = rows.iter().map(|q| q.role.as_str()).collect();
    let cats: Vec<Json<&Vec<String>>> = rows.iter().map(|q| Json(&q.categories)).collect();
    let company_ids: Vec<Option<&str>> = rows.iter().map(|q| q.company_id.as_deref()).collect();
    let added: Vec<chrono::DateTime<chrono::Utc>> = rows.iter().map(|q| q.added_at).collect();
    sqlx::query(
        r#"INSERT INTO questions
           (id, owner_id, text, source, role, categories, company_id, added_at)
           SELECT * FROM UNNEST(
               $1::text[],   $2::text[],   $3::text[],   $4::text[],
               $5::text[],   $6::jsonb[],  $7::text[],   $8::timestamptz[]
           )"#,
    )
    .bind(&ids)
    .bind(&owners)
    .bind(&texts)
    .bind(&sources)
    .bind(&roles)
    .bind(&cats)
    .bind(&company_ids)
    .bind(&added)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update(
    pool: &PgPool,
    owner_id: &str,
    id: &str,
    text: &str,
    role: Role,
    categories: &[String],
) -> Result<(), AppError> {
    sqlx::query(
        r#"UPDATE questions
           SET text = $3, role = $4, categories = $5
           WHERE owner_id = $1 AND id = $2"#,
    )
    .bind(owner_id)
    .bind(id)
    .bind(text)
    .bind(role.as_str())
    .bind(Json(categories.to_vec()))
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete(pool: &PgPool, owner_id: &str, id: &str) -> Result<(), AppError> {
    sqlx::query("DELETE FROM questions WHERE owner_id = $1 AND id = $2")
        .bind(owner_id)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Compatibility shim. Postgres FK CASCADE in migration 0001 handles
/// `questions.company_id → companies(id) ON DELETE CASCADE`, so by the
/// time `companies::delete` returns the questions are already gone.
/// Kept (returning 0) so route handlers don't need to be rewritten in
/// this sub-phase — callers should drop their invocations next round.
pub async fn delete_for_company(_pool: &PgPool, _company_id: &str) -> Result<u64, AppError> {
    Ok(0)
}

/// Pick one random unanswered question from a list. Used as the curator
/// fallback path inside `next_question`.
pub fn pick_random<'a>(pool: &'a [Question], seen: &HashSet<String>) -> Option<&'a Question> {
    let candidates: Vec<&Question> = pool.iter().filter(|q| !seen.contains(&q.id)).collect();
    candidates.choose(&mut rand::thread_rng()).copied()
}

// ── Compositions: classify + append ──────────────────────────────────────

/// Categorize + persist agent-suggested questions for a company, skipping
/// any whose text already exists. Returns the number of *new* questions
/// actually appended.
pub async fn append_skipping_existing(
    pool: &PgPool,
    or: &dyn LlmClient,
    owner_id: &str,
    company: &Company,
    candidates: Vec<String>,
    role: Role,
) -> Result<usize, AppError> {
    if candidates.is_empty() {
        return Ok(0);
    }
    let existing = list_for_company(pool, owner_id, &company.id).await?;
    let existing_texts: HashSet<String> = existing.iter().map(|q| q.text.clone()).collect();
    let new_questions: Vec<String> = candidates
        .into_iter()
        .filter(|q| !existing_texts.contains(q))
        .collect();
    let appended_n = new_questions.len();
    if appended_n > 0 {
        categorize_and_append(
            pool,
            or,
            owner_id,
            new_questions,
            QuestionSource::Agent,
            role,
            Some(company.id.clone()),
        )
        .await?;
    }
    Ok(appended_n)
}

/// Tag a batch of question texts via the lite_model classifier and persist
/// them.
pub async fn categorize_and_append(
    pool: &PgPool,
    openrouter: &dyn LlmClient,
    owner_id: &str,
    texts: Vec<String>,
    source: QuestionSource,
    role: Role,
    company_id: Option<String>,
) -> Result<usize, AppError> {
    if texts.is_empty() {
        return Ok(0);
    }
    let settings = settings_store::load(pool).await?;
    let canonical = category_store::list_all(pool).await?;
    let tags =
        categorize::classify_batch(openrouter, &settings.lite_model, &texts, &canonical).await;
    let by_text: std::collections::HashMap<String, Vec<String>> =
        texts.iter().cloned().zip(tags.into_iter()).collect();
    append(pool, owner_id, texts, source, role, company_id, |t| {
        by_text.get(t).cloned().unwrap_or_default()
    })
    .await
}

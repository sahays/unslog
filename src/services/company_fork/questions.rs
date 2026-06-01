//! Sub-step: deep-copy every question tagged to the source company.
//!
//! All rows are minted with new `qst…` ids, repointed at the new
//! `company_id`, and re-owned to the forker. Source rows are untouched.
//! Bulk INSERT via `UNNEST` keeps the round-trip cost at one query
//! regardless of how big the source bank is.

use sqlx::{Postgres, Transaction};

use crate::error::AppError;
use crate::services::id_gen::{self, Kind};

/// Returns the count of copied rows for the log line.
pub(super) async fn fork_questions_for_company(
    tx: &mut Transaction<'_, Postgres>,
    source_company_id: &str,
    new_company_id: &str,
    new_owner_id: &str,
) -> Result<u64, AppError> {
    let source_rows = sqlx::query!(
        r#"SELECT text, source, role, categories, added_at
           FROM questions
           WHERE company_id = $1
           ORDER BY added_at ASC"#,
        source_company_id,
    )
    .fetch_all(&mut **tx)
    .await?;
    if source_rows.is_empty() {
        return Ok(0);
    }
    let ids: Vec<String> = (0..source_rows.len())
        .map(|_| id_gen::new(Kind::Question))
        .collect();
    let owners: Vec<&str> = source_rows.iter().map(|_| new_owner_id).collect();
    let texts: Vec<&str> = source_rows.iter().map(|r| r.text.as_str()).collect();
    let sources: Vec<&str> = source_rows.iter().map(|r| r.source.as_str()).collect();
    let roles: Vec<&str> = source_rows.iter().map(|r| r.role.as_str()).collect();
    let cats: Vec<sqlx::types::JsonValue> =
        source_rows.iter().map(|r| r.categories.clone()).collect();
    let company_ids: Vec<&str> = source_rows.iter().map(|_| new_company_id).collect();
    let added: Vec<chrono::DateTime<chrono::Utc>> =
        source_rows.iter().map(|r| r.added_at).collect();
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
    .execute(&mut **tx)
    .await?;
    Ok(ids.len() as u64)
}

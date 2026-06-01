//! Pool-facing CRUD on the `summaries` table. Pure DB code so the
//! `summary.rs` parent stays focused on the generate-and-save flow.

use std::collections::HashMap;

use sqlx::types::Json;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::Summary;
use crate::services::current_owner::TEMP_OWNER_ID;

/// SELECT clause shared by every full-row read.
const SUMMARY_SELECT: &str = r#"
    SELECT id, owner_id, session_id, company_id, narrative,
           strengths            AS "strengths!: Json<Vec<String>>",
           recurring_weaknesses AS "recurring_weaknesses!: Json<Vec<String>>",
           blind_spots          AS "blind_spots!: Json<Vec<String>>",
           company_fit_signal, created_at
    FROM summaries
"#;

/// Last `limit` summaries for `company_id`, newest first. Used both inside
/// the summary prompt itself (priors) and inside future critique calls
/// (carry-forward context). `exclude_session` skips one session id — used
/// when re-running end on a rolled-back session.
pub async fn recent_for_company(
    pool: &PgPool,
    company_id: &str,
    exclude_session: Option<&str>,
    limit: i64,
) -> Result<Vec<Summary>, AppError> {
    let sql = format!(
        "{SUMMARY_SELECT}
         WHERE owner_id = $1 AND company_id = $2
           AND ($3::text IS NULL OR session_id <> $3)
         ORDER BY created_at DESC
         LIMIT $4"
    );
    let rows: Vec<SummaryRow> = sqlx::query_as(&sql)
        .bind(TEMP_OWNER_ID)
        .bind(company_id)
        .bind(exclude_session)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(SummaryRow::into_summary).collect())
}

/// Lookup-by-session helper for the per-session view.
pub async fn for_session(pool: &PgPool, session_id: &str) -> Result<Option<Summary>, AppError> {
    let sql = format!("{SUMMARY_SELECT} WHERE owner_id = $1 AND session_id = $2");
    let row: Option<SummaryRow> = sqlx::query_as(&sql)
        .bind(TEMP_OWNER_ID)
        .bind(session_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(SummaryRow::into_summary))
}

/// Bulk-load summaries for a set of session ids in one query. Returns
/// `session_id → Summary`. Drives the company show page.
pub async fn by_session_ids(
    pool: &PgPool,
    session_ids: &[&str],
) -> Result<HashMap<String, Summary>, AppError> {
    if session_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let owned: Vec<String> = session_ids.iter().map(|s| (*s).to_string()).collect();
    let sql = format!("{SUMMARY_SELECT} WHERE owner_id = $1 AND session_id = ANY($2)");
    let rows: Vec<SummaryRow> = sqlx::query_as(&sql)
        .bind(TEMP_OWNER_ID)
        .bind(&owned)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let s = r.into_summary();
            (s.session_id.clone(), s)
        })
        .collect())
}

pub async fn insert(pool: &PgPool, summary: &Summary) -> Result<(), AppError> {
    sqlx::query(
        r#"INSERT INTO summaries
           (id, owner_id, session_id, company_id, narrative,
            strengths, recurring_weaknesses, blind_spots,
            company_fit_signal, created_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
    )
    .bind(&summary.id)
    .bind(&summary.owner_id)
    .bind(&summary.session_id)
    .bind(&summary.company_id)
    .bind(&summary.narrative)
    .bind(Json(&summary.strengths))
    .bind(Json(&summary.recurring_weaknesses))
    .bind(Json(&summary.blind_spots))
    .bind(&summary.company_fit_signal)
    .bind(summary.created_at)
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct SummaryRow {
    id: String,
    owner_id: String,
    session_id: String,
    company_id: String,
    narrative: String,
    strengths: Json<Vec<String>>,
    recurring_weaknesses: Json<Vec<String>>,
    blind_spots: Json<Vec<String>>,
    company_fit_signal: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl SummaryRow {
    fn into_summary(self) -> Summary {
        Summary {
            id: self.id,
            owner_id: self.owner_id,
            session_id: self.session_id,
            company_id: self.company_id,
            narrative: self.narrative,
            strengths: self.strengths.0,
            recurring_weaknesses: self.recurring_weaknesses.0,
            blind_spots: self.blind_spots.0,
            company_fit_signal: self.company_fit_signal,
            created_at: self.created_at,
        }
    }
}

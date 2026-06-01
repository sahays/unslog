//! Postgres reads the curator needs: recent summaries for weakness bias,
//! plus the recently-asked question id set for the exclusion filter.

use std::collections::HashSet;

use sqlx::types::Json;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{Role, Summary};

/// How far back to look for "recent" sessions when computing weakness bias
/// and the recently-asked exclusion list.
const RECENT_SESSIONS: i64 = 3;

pub(super) async fn recent_summaries(
    pool: &PgPool,
    owner_id: &str,
    selected_company_ids: &[String],
) -> Result<Vec<Summary>, AppError> {
    if selected_company_ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = r#"
        SELECT id, owner_id, session_id, company_id, narrative,
               strengths            AS "strengths!: Json<Vec<String>>",
               recurring_weaknesses AS "recurring_weaknesses!: Json<Vec<String>>",
               blind_spots          AS "blind_spots!: Json<Vec<String>>",
               company_fit_signal, created_at
        FROM summaries
        WHERE owner_id = $1 AND company_id = ANY($2)
        ORDER BY created_at DESC
        LIMIT $3
    "#;
    let rows: Vec<SummaryRow> = sqlx::query_as(sql)
        .bind(owner_id)
        .bind(selected_company_ids)
        .bind(RECENT_SESSIONS)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(SummaryRow::into_summary).collect())
}

/// Set of question ids the user has *recently* answered for the selected
/// companies + role. One SQL JOIN against sessions+evaluations, ordered so
/// only the last `RECENT_SESSIONS * 2` ended sessions contribute.
pub(super) async fn recently_asked_ids(
    pool: &PgPool,
    owner_id: &str,
    selected_company_ids: &[String],
    role: Role,
) -> Result<HashSet<String>, AppError> {
    if selected_company_ids.is_empty() {
        return Ok(HashSet::new());
    }
    let limit = RECENT_SESSIONS * 2;
    let rows = sqlx::query!(
        r#"
        SELECT DISTINCT e.question_id
        FROM evaluations e
        JOIN sessions s ON s.id = e.session_id
        WHERE e.owner_id = $1
          AND s.owner_id = $1
          AND s.company_id = ANY($2)
          AND s.role = $3
          AND s.status = 'ended'
          AND s.id IN (
              SELECT id FROM sessions
              WHERE owner_id = $1
                AND company_id = ANY($2)
                AND role = $3
                AND status = 'ended'
              ORDER BY started_at DESC
              LIMIT $4
          )
        "#,
        owner_id,
        selected_company_ids,
        role.as_str(),
        limit,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.question_id).collect())
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

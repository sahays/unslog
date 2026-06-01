//! Sub-step: load the public source row + INSERT the forked clone.
//!
//! Source loader uses `SELECT … FOR SHARE` so the row can't be deleted
//! out from under us mid-transaction. Insert copies `research_packet`
//! verbatim (JSONB), forces `is_public = FALSE`, and tags the name with
//! " (forked)" so the new row is recognisable in the owner's list.

use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};

use crate::error::AppError;

/// Subset of the source company that `fork_company_row` needs. We don't
/// reuse `models::Company` here because the embedded `ResearchPacket`
/// would force JSON re-serialization — copying the raw JSONB through
/// keeps the bit-equality test honest.
pub(super) struct SourceCompany {
    pub owner_id: String,
    pub name: String,
    pub role: String,
    pub canonical_role: String,
    pub research_packet: Option<sqlx::types::JsonValue>,
    #[allow(dead_code)]
    pub created_at: DateTime<Utc>,
    #[allow(dead_code)]
    pub updated_at: DateTime<Utc>,
}

/// Load the source row inside the open tx. Non-public rows or unknown
/// ids surface as `AppError::NotFound` — same response either way so a
/// non-public id can't be probed for existence.
pub(super) async fn load_public_source(
    tx: &mut Transaction<'_, Postgres>,
    source_id: &str,
) -> Result<SourceCompany, AppError> {
    let row = sqlx::query!(
        r#"SELECT owner_id, name, role, canonical_role,
                  research_packet AS "research_packet: sqlx::types::JsonValue",
                  created_at, updated_at
           FROM companies
           WHERE id = $1 AND is_public = TRUE
           FOR SHARE"#,
        source_id,
    )
    .fetch_optional(&mut **tx)
    .await?;
    let row = row.ok_or_else(|| AppError::NotFound(format!("company {source_id}")))?;
    Ok(SourceCompany {
        owner_id: row.owner_id,
        name: row.name,
        role: row.role,
        canonical_role: row.canonical_role,
        research_packet: row.research_packet,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// INSERT the cloned row. `is_public = FALSE` is hard-wired — forks
/// start private so the new owner can polish before re-sharing.
pub(super) async fn fork_company_row(
    tx: &mut Transaction<'_, Postgres>,
    source: &SourceCompany,
    new_id: &str,
    new_owner_id: &str,
) -> Result<(), AppError> {
    let new_name = format!("{} (forked)", source.name);
    sqlx::query(
        r#"INSERT INTO companies
           (id, owner_id, name, role, canonical_role, research_packet,
            is_public, created_at, updated_at)
           VALUES ($1, $2, $3, $4, $5, $6, FALSE, NOW(), NOW())"#,
    )
    .bind(new_id)
    .bind(new_owner_id)
    .bind(&new_name)
    .bind(&source.role)
    .bind(&source.canonical_role)
    .bind(source.research_packet.as_ref())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

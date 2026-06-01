//! Store for the Postgres `companies` table — target-company packets.
//!
//! Every read is owner-scoped. The handler layer threads `CurrentUser::id`
//! through.
//!
//! `research_packet` is JSONB on the Postgres side and serializes through
//! [`sqlx::types::Json`]. Owner-scoped DELETE cascades through the FK in
//! migration 0001 (questions, sessions, etc.).
//!
//! Read/write surface:
//! * `get` / `find_or_404` — single-row lookups.
//! * `list_sorted` — projected list page (no packet body shipped).
//! * `list_by_name` — picker dropdown.
//! * `find_by_ids` — bulk fetch by id set.
//! * `insert` / `update_packet` / `delete` — write paths.
//! * `count` / `names_by_ids` — home page summary.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::types::Json;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::company::ResearchPacket;
use crate::models::{Company, Role};

/// Columns shared by every full-row read. Centralized so a schema add only
/// edits one place. `research_packet` returns as JSONB → `Option<Json<...>>`.
const COMPANY_COLS: &str = r#"id, owner_id, name, role, canonical_role,
    research_packet AS "research_packet: Json<ResearchPacket>",
    is_public, created_at, updated_at"#;

#[derive(sqlx::FromRow)]
struct CompanyRow {
    id: String,
    owner_id: String,
    name: String,
    role: String,
    canonical_role: String,
    research_packet: Option<Json<ResearchPacket>>,
    is_public: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl CompanyRow {
    fn try_into_company(self) -> Result<Company, AppError> {
        let canonical_role = Role::parse(&self.canonical_role).ok_or_else(|| {
            AppError::Other(anyhow::anyhow!(
                "companies.canonical_role {:?} not in CHECK set",
                self.canonical_role
            ))
        })?;
        Ok(Company {
            id: self.id,
            owner_id: self.owner_id,
            name: self.name,
            role: self.role,
            canonical_role,
            research_packet: self.research_packet.map(|j| j.0),
            is_public: self.is_public,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

/// Projected row for the `/companies` list page. The full
/// `research_packet` is multi-KB per row and the list view only renders
/// name / role / dates / a "packet ready" badge — so we project to a
/// tight row type and replace the packet with a boolean.
pub struct CompanyListRow {
    pub id: String,
    pub name: String,
    pub role: String,
    pub canonical_role: Role,
    pub created_at: DateTime<Utc>,
    pub has_packet: bool,
}

// ── Single-row reads ─────────────────────────────────────────────────────

pub async fn get(pool: &PgPool, owner_id: &str, id: &str) -> Result<Option<Company>, AppError> {
    let sql = format!("SELECT {COMPANY_COLS} FROM companies WHERE owner_id = $1 AND id = $2");
    let row: Option<CompanyRow> = sqlx::query_as(&sql)
        .bind(owner_id)
        .bind(id)
        .fetch_optional(pool)
        .await?;
    row.map(CompanyRow::try_into_company).transpose()
}

pub async fn find_or_404(pool: &PgPool, owner_id: &str, id: &str) -> Result<Company, AppError> {
    get(pool, owner_id, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("company {id}")))
}

// ── List / projection reads ──────────────────────────────────────────────

/// All companies, newest-first. Projected for the list page — no
/// research-packet body. `has_packet` is derived server-side via the
/// `research_packet IS NOT NULL` check so the multi-KB JSONB stays on
/// the database side.
pub async fn list_sorted(pool: &PgPool, owner_id: &str) -> Result<Vec<CompanyListRow>, AppError> {
    let rows = sqlx::query!(
        r#"
        SELECT id, name, role, canonical_role, created_at,
               (research_packet IS NOT NULL) AS "has_packet!"
        FROM companies
        WHERE owner_id = $1
        ORDER BY updated_at DESC
        "#,
        owner_id,
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            let canonical_role = Role::parse(&r.canonical_role).ok_or_else(|| {
                AppError::Other(anyhow::anyhow!(
                    "companies.canonical_role {:?} not in CHECK set",
                    r.canonical_role
                ))
            })?;
            Ok(CompanyListRow {
                id: r.id,
                name: r.name,
                role: r.role,
                canonical_role,
                created_at: r.created_at,
                has_packet: r.has_packet,
            })
        })
        .collect()
}

/// All companies for the owner, sorted by name. Drives the `/practice`
/// picker (full Company rows including the packet, since the picker
/// needs canonical_role for filtering).
pub async fn list_by_name(pool: &PgPool, owner_id: &str) -> Result<Vec<Company>, AppError> {
    let sql = format!(
        "SELECT {COMPANY_COLS} FROM companies
         WHERE owner_id = $1
         ORDER BY name ASC"
    );
    let rows: Vec<CompanyRow> = sqlx::query_as(&sql).bind(owner_id).fetch_all(pool).await?;
    rows.into_iter().map(CompanyRow::try_into_company).collect()
}

/// Bulk fetch full company rows for a set of ids in one query.
pub async fn find_by_ids(
    pool: &PgPool,
    owner_id: &str,
    ids: &[String],
) -> Result<Vec<Company>, AppError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT {COMPANY_COLS} FROM companies
         WHERE owner_id = $1 AND id = ANY($2)"
    );
    let rows: Vec<CompanyRow> = sqlx::query_as(&sql)
        .bind(owner_id)
        .bind(ids)
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(CompanyRow::try_into_company).collect()
}

/// Bulk-load company names for a set of ids. Returns `id → name`.
pub async fn names_by_ids(
    pool: &PgPool,
    owner_id: &str,
    ids: &[&str],
) -> Result<HashMap<String, String>, AppError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let ids_owned: Vec<String> = ids.iter().map(|s| (*s).to_string()).collect();
    let rows = sqlx::query!(
        r#"
        SELECT id, name FROM companies
        WHERE owner_id = $1 AND id = ANY($2)
        "#,
        owner_id,
        &ids_owned,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| (r.id, r.name)).collect())
}

pub async fn count(pool: &PgPool, owner_id: &str) -> Result<u64, AppError> {
    let n: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) AS \"count!\" FROM companies WHERE owner_id = $1",
        owner_id,
    )
    .fetch_one(pool)
    .await?;
    Ok(n as u64)
}

// ── Writes ───────────────────────────────────────────────────────────────

/// Persist a freshly-built company row. The caller mints `company.id`
/// via `models::Company::new` (which uses `id_gen::new(Kind::Company)`)
/// and sets `company.owner_id` to the current owner.
pub async fn insert(pool: &PgPool, company: &Company) -> Result<(), AppError> {
    let packet = company.research_packet.as_ref().map(|p| Json(p.clone()));
    sqlx::query(
        r#"INSERT INTO companies
           (id, owner_id, name, role, canonical_role, research_packet,
            is_public, created_at, updated_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"#,
    )
    .bind(&company.id)
    .bind(&company.owner_id)
    .bind(&company.name)
    .bind(&company.role)
    .bind(company.canonical_role.as_str())
    .bind(packet)
    .bind(company.is_public)
    .bind(company.created_at)
    .bind(company.updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Refresh a company's research packet. Owner-scoped; bumps `updated_at`.
pub async fn update_packet(
    pool: &PgPool,
    owner_id: &str,
    company_id: &str,
    packet: &ResearchPacket,
) -> Result<(), AppError> {
    sqlx::query(
        r#"UPDATE companies
           SET research_packet = $3,
               updated_at      = NOW()
           WHERE owner_id = $1 AND id = $2"#,
    )
    .bind(owner_id)
    .bind(company_id)
    .bind(Json(packet.clone()))
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete a company row. The Postgres FK CASCADE drops dependent
/// questions / sessions / evaluations / summaries (see migration 0001).
pub async fn delete(pool: &PgPool, owner_id: &str, company_id: &str) -> Result<(), AppError> {
    sqlx::query("DELETE FROM companies WHERE owner_id = $1 AND id = $2")
        .bind(owner_id)
        .bind(company_id)
        .execute(pool)
        .await?;
    Ok(())
}

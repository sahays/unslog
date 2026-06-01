//! Owner-only publish/unpublish toggle + cross-tenant browse for public
//! companies.
//!
//! Each row in `companies` carries an `is_public` flag (migration 0006).
//! Flipping it is the publish/unpublish primitive — only the owner can
//! do so, and `set_company_public` enforces that with a single
//! `UPDATE … WHERE id = $1 AND owner_id = $2`. A 0-row update is
//! returned as [`AppError::NotFound`] so non-owners get the same response
//! shape as if the company didn't exist (no existence leak).
//!
//! `list_public_companies` joins `companies` with `users` so the browse
//! page can show "from `<owner_label>`" attribution without making the
//! caller fan out N extra `users` lookups.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::Role;

/// Card rendered on `/browse`. Projected — the full `research_packet`
/// stays on the DB side; the browse page only shows name + role + owner
/// + last-updated.
pub struct PublicCompanyCard {
    pub id: String,
    pub name: String,
    pub role: String,
    pub canonical_role: Role,
    pub owner_id: String,
    pub owner_label: String,
    pub updated_at: DateTime<Utc>,
}

/// Persistence seam consumed by the sharing route handlers. Production:
/// [`PgSharingDeps`]; tests: `MockSharingDeps`.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait SharingDeps: Send + Sync {
    /// Flip `companies.is_public` for an owner-owned row. Returns
    /// `AppError::NotFound` if the (owner_id, company_id) pair doesn't
    /// match a row — same shape as non-owner access, on purpose.
    async fn set_company_public(
        &self,
        owner_id: &str,
        company_id: &str,
        is_public: bool,
    ) -> Result<(), AppError>;

    /// All `is_public = TRUE` companies across every owner, newest first.
    async fn list_public_companies(&self) -> Result<Vec<PublicCompanyCard>, AppError>;
}

/// Production [`SharingDeps`] backed by a Postgres pool.
pub struct PgSharingDeps<'a> {
    pub pool: &'a PgPool,
}

#[async_trait]
impl<'a> SharingDeps for PgSharingDeps<'a> {
    async fn set_company_public(
        &self,
        owner_id: &str,
        company_id: &str,
        is_public: bool,
    ) -> Result<(), AppError> {
        let rows = sqlx::query(
            r#"UPDATE companies
               SET is_public = $1, updated_at = NOW()
               WHERE id = $2 AND owner_id = $3"#,
        )
        .bind(is_public)
        .bind(company_id)
        .bind(owner_id)
        .execute(self.pool)
        .await?
        .rows_affected();
        if rows == 0 {
            return Err(AppError::NotFound(format!("company {company_id}")));
        }
        Ok(())
    }

    async fn list_public_companies(&self) -> Result<Vec<PublicCompanyCard>, AppError> {
        let rows = sqlx::query!(
            r#"
            SELECT c.id, c.name, c.role, c.canonical_role,
                   c.owner_id, c.updated_at,
                   u.label AS owner_label
            FROM companies c
            JOIN users u ON u.id = c.owner_id
            WHERE c.is_public = TRUE
            ORDER BY c.updated_at DESC
            "#,
        )
        .fetch_all(self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                let canonical_role = Role::parse(&r.canonical_role).ok_or_else(|| {
                    AppError::Other(anyhow::anyhow!(
                        "companies.canonical_role {:?} not in CHECK set",
                        r.canonical_role
                    ))
                })?;
                Ok(PublicCompanyCard {
                    id: r.id,
                    name: r.name,
                    role: r.role,
                    canonical_role,
                    owner_id: r.owner_id,
                    owner_label: r.owner_label,
                    updated_at: r.updated_at,
                })
            })
            .collect()
    }
}

// ── Pool-facing convenience wrappers ────────────────────────────────────
//
// Route handlers prefer these over building a `PgSharingDeps` by hand so
// the call site reads like pseudocode.

pub async fn set_company_public(
    pool: &PgPool,
    owner_id: &str,
    company_id: &str,
    is_public: bool,
) -> Result<(), AppError> {
    PgSharingDeps { pool }
        .set_company_public(owner_id, company_id, is_public)
        .await
}

pub async fn list_public_companies(pool: &PgPool) -> Result<Vec<PublicCompanyCard>, AppError> {
    PgSharingDeps { pool }.list_public_companies().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockall::predicate::*;

    #[tokio::test]
    async fn case_mock_set_public_calls_through_with_owner_scope() {
        let mut deps = MockSharingDeps::new();
        deps.expect_set_company_public()
            .with(eq("usralice0"), eq("comabcdef"), eq(true))
            .times(1)
            .returning(|_, _, _| Ok(()));
        deps.set_company_public("usralice0", "comabcdef", true)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn case_mock_set_public_propagates_not_found() {
        let mut deps = MockSharingDeps::new();
        deps.expect_set_company_public()
            .returning(|_, _, _| Err(AppError::NotFound("company comabcdef".into())));
        let err = deps
            .set_company_public("usrbob000", "comabcdef", true)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
    }
}

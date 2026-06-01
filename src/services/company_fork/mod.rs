//! Deep-copy fork of a public company into the caller's namespace.
//!
//! `fork(pool, source_id, new_owner_id)` runs the entire flow inside one
//! Postgres transaction so the new company + cloned question bank + audit
//! row either all land or none do. Sub-steps live in their own files so
//! each one stays small and focused:
//!
//!   * [`company`] — INSERT new `companies` row (research packet copied
//!     verbatim; `is_public = FALSE`; name suffix " (forked)").
//!   * [`questions`] — bulk-copy all source questions, rewriting `id` and
//!     `company_id`.
//!   * [`audit`] — INSERT the `forks` audit row.
//!
//! Guard rails:
//!
//!   * The source row must be `is_public = TRUE` — anything else
//!     surfaces as `AppError::NotFound` (no existence leak).
//!   * Owners cannot fork their own row — `AppError::BadRequest`.

mod audit;
mod company;
mod questions;

use sqlx::PgPool;

use crate::error::AppError;
use crate::services::id_gen::{self, Kind};

/// Deep-copy a public company into `new_owner_id`'s namespace. Returns
/// the freshly minted company id. The whole flow is one transaction.
pub async fn fork(pool: &PgPool, source_id: &str, new_owner_id: &str) -> Result<String, AppError> {
    let mut tx = pool.begin().await?;
    let source = company::load_public_source(&mut tx, source_id).await?;
    if source.owner_id == new_owner_id {
        return Err(AppError::BadRequest(
            "you cannot fork your own company".into(),
        ));
    }
    let new_id = id_gen::new(Kind::Company);
    company::fork_company_row(&mut tx, &source, &new_id, new_owner_id).await?;
    let copied_questions =
        questions::fork_questions_for_company(&mut tx, source_id, &new_id, new_owner_id).await?;
    audit::write_audit(&mut tx, &new_id, source_id, new_owner_id).await?;
    tx.commit().await?;
    tracing::info!(
        event = "share.company.fork",
        source_company_id = %source_id,
        new_company_id = %new_id,
        new_owner_id = %new_owner_id,
        questions_copied = copied_questions,
        "company forked",
    );
    Ok(new_id)
}

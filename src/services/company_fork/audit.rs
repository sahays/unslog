//! Sub-step: write the `forks` audit row.
//!
//! One row per successful fork. `source_kind = 'company'` today; the
//! column is TEXT + CHECK so future kinds (stories, pitches, prompts)
//! can extend the audit table without a schema change.

use sqlx::{Postgres, Transaction};

use crate::error::AppError;
use crate::services::id_gen::{self, Kind};

pub(super) async fn write_audit(
    tx: &mut Transaction<'_, Postgres>,
    new_id: &str,
    source_id: &str,
    new_owner_id: &str,
) -> Result<(), AppError> {
    let fork_id = id_gen::new(Kind::Fork);
    sqlx::query(
        r#"INSERT INTO forks (id, source_kind, source_id, new_id, owner_id)
           VALUES ($1, 'company', $2, $3, $4)"#,
    )
    .bind(&fork_id)
    .bind(source_id)
    .bind(new_id)
    .bind(new_owner_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

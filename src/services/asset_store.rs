//! Store for the Postgres `assets` table — uploaded book / resume PDFs.
//!
//! Every read is scoped by `owner_id`. `kind = 'book'` rows are pinned to
//! the master user by a trigger (migration 0003) so the critique flow
//! sees the same book regardless of which user is logged in. Resumes are
//! per-user. `set_primary` runs in a transaction so the partial unique
//! index `assets_one_primary_per_owner_kind_uidx` is never violated.
//!
//! Primary-management logic flows through [`AssetSource`] so the per-kind
//! scoping rules are unit-testable without a live DB.

use async_trait::async_trait;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{Asset, AssetKind, ExtractionStatus};
use crate::services::current_owner::TEMP_OWNER_ID;

#[path = "asset_store_row.rs"]
mod row;
use row::{AssetRow, ASSET_COLS};

/// Persistence seam for the primary-management logic. Production impl is
/// [`PgAssetSource`]; tests inject `MockAssetSource`.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait AssetSource: Send + Sync {
    /// Most-recently-uploaded row of a given kind whose id is not
    /// `exclude_id`. Used to pick a replacement primary on delete.
    async fn find_newest_of_kind_excluding(
        &self,
        kind: AssetKind,
        exclude_id: &str,
    ) -> Result<Option<Asset>, AppError>;
    async fn set_primary_flag(&self, id: &str, value: bool) -> Result<u64, AppError>;
    /// Clear `primary` on every asset of `kind` whose id is not `keep_id`.
    async fn clear_primary_in_kind_excluding(
        &self,
        kind: AssetKind,
        keep_id: &str,
    ) -> Result<(), AppError>;
    async fn delete_by_id(&self, id: &str) -> Result<(), AppError>;
    async fn find_by_id(&self, id: &str) -> Result<Option<Asset>, AppError>;
}

/// Production `AssetSource` over a `&PgPool`.
pub struct PgAssetSource<'a> {
    pub pool: &'a PgPool,
}

#[async_trait]
impl<'a> AssetSource for PgAssetSource<'a> {
    async fn find_newest_of_kind_excluding(
        &self,
        kind: AssetKind,
        exclude_id: &str,
    ) -> Result<Option<Asset>, AppError> {
        let sql = format!(
            "SELECT {ASSET_COLS}
             FROM assets
             WHERE owner_id = $1 AND kind = $2 AND id <> $3
             ORDER BY uploaded_at DESC
             LIMIT 1"
        );
        let row: Option<AssetRow> = sqlx::query_as(&sql)
            .bind(TEMP_OWNER_ID)
            .bind(kind.as_str())
            .bind(exclude_id)
            .fetch_optional(self.pool)
            .await?;
        row.map(AssetRow::try_into_asset).transpose()
    }

    async fn set_primary_flag(&self, id: &str, value: bool) -> Result<u64, AppError> {
        let res = sqlx::query(r#"UPDATE assets SET "primary" = $2 WHERE id = $1"#)
            .bind(id)
            .bind(value)
            .execute(self.pool)
            .await?;
        Ok(res.rows_affected())
    }

    async fn clear_primary_in_kind_excluding(
        &self,
        kind: AssetKind,
        keep_id: &str,
    ) -> Result<(), AppError> {
        sqlx::query(
            r#"UPDATE assets SET "primary" = FALSE
               WHERE owner_id = $1 AND kind = $2 AND id <> $3"#,
        )
        .bind(TEMP_OWNER_ID)
        .bind(kind.as_str())
        .bind(keep_id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    async fn delete_by_id(&self, id: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM assets WHERE id = $1")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    async fn find_by_id(&self, id: &str) -> Result<Option<Asset>, AppError> {
        let sql = format!("SELECT {ASSET_COLS} FROM assets WHERE id = $1");
        let row: Option<AssetRow> = sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(self.pool)
            .await?;
        row.map(AssetRow::try_into_asset).transpose()
    }
}

// ── Plain lookups (no trait — direct pool) ──────────────────────────────

// Until Phase 1 lands a real `current_user` extractor, every read +
// write uses the master owner. Book reads in particular must always pin
// to the master because the `assets_book_must_be_master_trg` trigger
// rejects non-master inserts of `kind = 'book'`.

pub async fn get(pool: &PgPool, id: &str) -> Result<Option<Asset>, AppError> {
    PgAssetSource { pool }.find_by_id(id).await
}

pub async fn find_or_404(pool: &PgPool, id: &str) -> Result<Asset, AppError> {
    get(pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("asset {id}")))
}

/// Primary asset of a given kind, if any. Backed by the partial unique
/// index `assets_one_primary_per_owner_kind_uidx`.
pub async fn find_primary_by_kind(
    pool: &PgPool,
    kind: AssetKind,
) -> Result<Option<Asset>, AppError> {
    let sql = format!(
        r#"SELECT {ASSET_COLS} FROM assets
           WHERE owner_id = $1 AND kind = $2 AND "primary" = TRUE LIMIT 1"#
    );
    let row: Option<AssetRow> = sqlx::query_as(&sql)
        .bind(TEMP_OWNER_ID)
        .bind(kind.as_str())
        .fetch_optional(pool)
        .await?;
    row.map(AssetRow::try_into_asset).transpose()
}

/// All assets visible to the caller, newest-uploaded first. Drives the
/// library page.
pub async fn list_sorted(pool: &PgPool) -> Result<Vec<Asset>, AppError> {
    let sql = format!(
        "SELECT {ASSET_COLS}
         FROM assets
         WHERE owner_id = $1
         ORDER BY uploaded_at DESC"
    );
    let rows: Vec<AssetRow> = sqlx::query_as(&sql)
        .bind(TEMP_OWNER_ID)
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(AssetRow::try_into_asset).collect()
}

pub async fn insert(pool: &PgPool, asset: &Asset) -> Result<(), AppError> {
    sqlx::query(
        r#"INSERT INTO assets
           (id, owner_id, name, kind, "primary",
            original_filename, original_path, extracted_path,
            extraction_status, extraction_error, uploaded_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#,
    )
    .bind(&asset.id)
    .bind(&asset.owner_id)
    .bind(&asset.name)
    .bind(asset.kind.as_str())
    .bind(asset.primary)
    .bind(&asset.original_filename)
    .bind(&asset.original_path)
    .bind(&asset.extracted_path)
    .bind(asset.extraction_status.as_str())
    .bind(&asset.extraction_error)
    .bind(asset.uploaded_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete `asset` while preserving the "one primary per (owner, kind) if
/// any rows of that kind exist" invariant. When `asset` is the current
/// primary, promote the next-most-recent peer of the same kind first,
/// then delete. A crash between writes leaves a brief two-primary window
/// (recoverable through `find_primary_by_kind`'s `LIMIT 1`) — never zero,
/// which is the failure mode that breaks the critique flow.
pub async fn delete(pool: &PgPool, asset: &Asset) -> Result<(), AppError> {
    delete_via(&PgAssetSource { pool }, asset).await
}

pub async fn delete_via<S: AssetSource + ?Sized>(src: &S, asset: &Asset) -> Result<(), AppError> {
    if asset.primary {
        let _ = promote_next_in_kind_excluding(src, asset.kind, &asset.id).await?;
    }
    src.delete_by_id(&asset.id).await
}

async fn promote_next_in_kind_excluding<S: AssetSource + ?Sized>(
    src: &S,
    kind: AssetKind,
    exclude_id: &str,
) -> Result<bool, AppError> {
    let Some(next) = src.find_newest_of_kind_excluding(kind, exclude_id).await? else {
        return Ok(false);
    };
    let _ = src.set_primary_flag(&next.id, true).await?;
    Ok(true)
}

/// Flip `primary: true` to `id`, scoped to the asset's own kind. Demote
/// peers first then promote the target — wrapped in a transaction so the
/// partial unique index never sees two primaries for `(owner, kind)`.
/// Returns 404 if `id` doesn't exist.
pub async fn set_primary(pool: &PgPool, id: &str) -> Result<(), AppError> {
    let target = find_or_404(pool, id).await?;
    let mut tx = pool.begin().await?;
    sqlx::query(
        r#"UPDATE assets SET "primary" = FALSE
           WHERE owner_id = $1 AND kind = $2 AND id <> $3"#,
    )
    .bind(&target.owner_id)
    .bind(target.kind.as_str())
    .bind(id)
    .execute(&mut *tx)
    .await?;
    let res = sqlx::query(r#"UPDATE assets SET "primary" = TRUE WHERE id = $1"#)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if res.rows_affected() == 0 {
        tx.rollback().await?;
        return Err(AppError::NotFound(format!("asset {id}")));
    }
    tx.commit().await?;
    Ok(())
}

/// Same as [`set_primary`] but generic over [`AssetSource`] for test
/// injection. Mirrors the trait-driven order: promote target first, then
/// clear peers — the mocks only see logical calls, not a real index.
pub async fn set_primary_via<S: AssetSource + ?Sized>(src: &S, id: &str) -> Result<(), AppError> {
    let target = src
        .find_by_id(id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("asset {id}")))?;
    let matched = src.set_primary_flag(id, true).await?;
    if matched == 0 {
        return Err(AppError::NotFound(format!("asset {id}")));
    }
    src.clear_primary_in_kind_excluding(target.kind, id).await?;
    Ok(())
}

/// Persist an extraction result onto an existing asset row.
pub async fn update_extraction(
    pool: &PgPool,
    id: &str,
    status: ExtractionStatus,
    extracted_path: Option<String>,
    extraction_error: Option<String>,
) -> Result<(), AppError> {
    sqlx::query(
        r#"UPDATE assets
           SET extraction_status = $2,
               extracted_path = $3,
               extraction_error = $4
           WHERE id = $1"#,
    )
    .bind(id)
    .bind(status.as_str())
    .bind(extracted_path)
    .bind(extraction_error)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
#[path = "asset_store_tests.rs"]
mod tests;

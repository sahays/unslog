//! Store for the `assets` collection — uploaded book / reference / resume PDFs.
//!
//! Read/write surface:
//! * `get` / `find_or_404` — lookups.
//! * `find_primary_by_kind` — the one row with `{kind, primary: true}`, if any.
//! * `list_sorted` — newest-first list for the library page.
//! * `insert` — single-row write (delete goes through [`delete`] below
//!   for primary-promotion-safe removal).
//! * `set_primary` — flip primary by promoting the target first, then
//!   demoting peers **of the same kind**. The brief window may have two
//!   primaries within a kind, never zero; downstream consumers
//!   (`find_primary_by_kind` uses `find_one`) tolerate the two-primary window.
//! * `delete` — drops a row and, when the deleted row was primary, promotes
//!   the next-most-recent peer **of the same kind** to keep one primary
//!   alive per non-empty kind.
//!
//! The primary-management functions go through the [`AssetSource`] trait so
//! the per-kind scoping logic is unit-testable without a live Mongo.

use async_trait::async_trait;
use futures::TryStreamExt;
use mongodb::options::{FindOneOptions, FindOptions};
use mongodb::Database;

use crate::error::AppError;
use crate::models::{Asset, AssetKind};

/// Persistence seam for the primary-management logic. Production impl is
/// `mongodb::Database`; tests inject `MockAssetSource`.
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

#[async_trait]
impl AssetSource for Database {
    async fn find_newest_of_kind_excluding(
        &self,
        kind: AssetKind,
        exclude_id: &str,
    ) -> Result<Option<Asset>, AppError> {
        Ok(self
            .collection::<Asset>(Asset::COLLECTION)
            .find_one(bson::doc! {
                "kind": kind.as_str(),
                "_id": { "$ne": exclude_id },
            })
            .with_options(
                FindOneOptions::builder()
                    .sort(bson::doc! { "uploaded_at": -1 })
                    .build(),
            )
            .await?)
    }

    async fn set_primary_flag(&self, id: &str, value: bool) -> Result<u64, AppError> {
        let res = self
            .collection::<Asset>(Asset::COLLECTION)
            .update_one(
                bson::doc! { "_id": id },
                bson::doc! { "$set": { "primary": value } },
            )
            .await?;
        Ok(res.matched_count)
    }

    async fn clear_primary_in_kind_excluding(
        &self,
        kind: AssetKind,
        keep_id: &str,
    ) -> Result<(), AppError> {
        self.collection::<Asset>(Asset::COLLECTION)
            .update_many(
                bson::doc! {
                    "kind": kind.as_str(),
                    "_id": { "$ne": keep_id },
                },
                bson::doc! { "$set": { "primary": false } },
            )
            .await?;
        Ok(())
    }

    async fn delete_by_id(&self, id: &str) -> Result<(), AppError> {
        self.collection::<Asset>(Asset::COLLECTION)
            .delete_one(bson::doc! { "_id": id })
            .await?;
        Ok(())
    }

    async fn find_by_id(&self, id: &str) -> Result<Option<Asset>, AppError> {
        Ok(self
            .collection::<Asset>(Asset::COLLECTION)
            .find_one(bson::doc! { "_id": id })
            .await?)
    }
}

// ── Plain lookups (no trait — direct Mongo) ──────────────────────────────

pub async fn get(db: &Database, id: &str) -> Result<Option<Asset>, AppError> {
    Ok(db
        .collection::<Asset>(Asset::COLLECTION)
        .find_one(bson::doc! { "_id": id })
        .await?)
}

pub async fn find_or_404(db: &Database, id: &str) -> Result<Asset, AppError> {
    get(db, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("asset {id}")))
}

/// Primary asset of a given kind, if any. Backed by the `kind_primary`
/// compound index — cheap to call per coach turn.
pub async fn find_primary_by_kind(
    db: &Database,
    kind: AssetKind,
) -> Result<Option<Asset>, AppError> {
    Ok(db
        .collection::<Asset>(Asset::COLLECTION)
        .find_one(bson::doc! { "kind": kind.as_str(), "primary": true })
        .await?)
}

/// All assets, newest-uploaded first. Drives the library page.
pub async fn list_sorted(db: &Database) -> Result<Vec<Asset>, AppError> {
    let opts = FindOptions::builder()
        .sort(bson::doc! { "uploaded_at": -1 })
        .build();
    let cursor = db
        .collection::<Asset>(Asset::COLLECTION)
        .find(bson::doc! {})
        .with_options(opts)
        .await?;
    Ok(cursor.try_collect().await?)
}

pub async fn insert(db: &Database, asset: &Asset) -> Result<(), AppError> {
    db.collection::<Asset>(Asset::COLLECTION)
        .insert_one(asset)
        .await?;
    Ok(())
}

/// Delete `asset` while preserving the "one primary per kind if any rows of
/// that kind exist" invariant. When `asset` is the current primary, we
/// **promote the next-most-recent peer of the same kind first**, then
/// delete the old primary. A crash between the two writes leaves a brief
/// two-primary window within the kind — never zero, which is the failure
/// mode that breaks the critique flow. When no peer of the same kind
/// remains, zero primaries for that kind is the correct state.
pub async fn delete(db: &Database, asset: &Asset) -> Result<(), AppError> {
    delete_via(db, asset).await
}

/// Same as [`delete`] but generic over [`AssetSource`] for test injection.
pub async fn delete_via<S: AssetSource + ?Sized>(src: &S, asset: &Asset) -> Result<(), AppError> {
    if asset.primary {
        let _ = promote_next_in_kind_excluding(src, asset.kind, &asset.id).await?;
    }
    src.delete_by_id(&asset.id).await
}

/// Promote the most-recently-uploaded asset of `kind` whose id is **not**
/// `exclude_id` to primary. Returns whether a promotion happened.
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

/// Flip `primary: true` to `id`, scoped to the asset's own kind. Ordering
/// matters: we **promote the target first**, then demote every other row of
/// the same kind in a single `update_many`. A crash between the two writes
/// leaves the kind with two primaries — recoverable, because
/// `find_primary_by_kind` uses `find_one` and accepts whichever row Mongo
/// returns. The inverse order (demote-all then promote-one) could leave
/// zero primaries within the kind, which the consuming caches can't
/// recover from. Returns 404 if `id` doesn't exist.
pub async fn set_primary(db: &Database, id: &str) -> Result<(), AppError> {
    set_primary_via(db, id).await
}

/// Same as [`set_primary`] but generic over [`AssetSource`] for test injection.
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
    db: &Database,
    id: &str,
    status: crate::models::ExtractionStatus,
    extracted_path: Option<String>,
    extraction_error: Option<String>,
) -> Result<(), AppError> {
    db.collection::<Asset>(Asset::COLLECTION)
        .update_one(
            bson::doc! { "_id": id },
            bson::doc! {
                "$set": {
                    "extraction_status": bson::to_bson(&status)?,
                    "extracted_path": extracted_path,
                    "extraction_error": extraction_error,
                }
            },
        )
        .await?;
    Ok(())
}

#[cfg(test)]
#[path = "asset_store_tests.rs"]
mod tests;

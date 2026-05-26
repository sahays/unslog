//! Store for the `assets` collection — uploaded book / reference PDFs.
//!
//! Read/write surface:
//! * `get` / `find_or_404` — lookups.
//! * `find_primary` — the one row with `primary: true`, if any.
//! * `list_sorted` — newest-first list for the library page.
//! * `count` / `count_primary` — cheap presence checks for the home page
//!   and the active-session view.
//! * `insert` / `delete_one` — basic CRUD.
//! * `set_primary` — flip primary by promoting the target first, then
//!   demoting peers. The brief window may have two primaries, never zero;
//!   downstream consumers (`find_primary` uses `find_one`) tolerate the
//!   two-primary window.
//! * `promote_newest_to_primary` — used by `delete` to keep a primary row
//!   alive when the one we just dropped was the primary.

use futures::TryStreamExt;
use mongodb::options::{FindOneOptions, FindOptions};
use mongodb::Database;

use crate::error::AppError;
use crate::models::Asset;

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

pub async fn find_primary(db: &Database) -> Result<Option<Asset>, AppError> {
    Ok(db
        .collection::<Asset>(Asset::COLLECTION)
        .find_one(bson::doc! { "primary": true })
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

pub async fn count(db: &Database) -> Result<u64, AppError> {
    Ok(db
        .collection::<Asset>(Asset::COLLECTION)
        .count_documents(bson::doc! {})
        .await?)
}

pub async fn count_primary(db: &Database) -> Result<u64, AppError> {
    Ok(db
        .collection::<Asset>(Asset::COLLECTION)
        .count_documents(bson::doc! { "primary": true })
        .await?)
}

pub async fn insert(db: &Database, asset: &Asset) -> Result<(), AppError> {
    db.collection::<Asset>(Asset::COLLECTION)
        .insert_one(asset)
        .await?;
    Ok(())
}

/// Raw row delete (no primary-promotion logic). Used by [`delete`] and
/// by callers that already know the target isn't the current primary.
pub async fn delete_one(db: &Database, id: &str) -> Result<(), AppError> {
    db.collection::<Asset>(Asset::COLLECTION)
        .delete_one(bson::doc! { "_id": id })
        .await?;
    Ok(())
}

/// Delete `asset` while preserving the "one primary if any rows exist"
/// invariant. When `asset` is the current primary, we **promote the
/// next-most-recent row first**, then delete the old primary. A crash
/// between the two writes leaves a brief two-primary window — never
/// zero, which is the failure mode that breaks the critique flow.
/// When the library is empty after the delete, zero primaries is the
/// correct state.
pub async fn delete(db: &Database, asset: &Asset) -> Result<(), AppError> {
    if asset.primary {
        let _ = promote_next_excluding(db, &asset.id).await?;
    }
    delete_one(db, &asset.id).await
}

/// Promote the most-recently-uploaded asset whose id is **not**
/// `exclude_id` to primary. Returns whether a promotion happened.
/// Internal helper for [`delete`]; the two-step ordering matters.
async fn promote_next_excluding(db: &Database, exclude_id: &str) -> Result<bool, AppError> {
    let coll = db.collection::<Asset>(Asset::COLLECTION);
    let next = coll
        .find_one(bson::doc! { "_id": { "$ne": exclude_id } })
        .with_options(
            FindOneOptions::builder()
                .sort(bson::doc! { "uploaded_at": -1 })
                .build(),
        )
        .await?;
    let Some(next) = next else {
        return Ok(false);
    };
    coll.update_one(
        bson::doc! { "_id": &next.id },
        bson::doc! { "$set": { "primary": true } },
    )
    .await?;
    Ok(true)
}

/// Flip `primary: true` to `id`. Ordering matters: we **promote the
/// target first**, then demote every other row in a single
/// `update_many`. A crash between the two writes leaves the system with
/// two primaries — recoverable, because the `find_primary` consumer
/// uses `find_one({primary: true})` and accepts whichever row Mongo
/// returns. The inverse order (demote-all then promote-one) could leave
/// zero primaries, which the critique flow can't recover from.
/// Returns 404 if `id` doesn't exist.
pub async fn set_primary(db: &Database, id: &str) -> Result<(), AppError> {
    let coll = db.collection::<Asset>(Asset::COLLECTION);
    let res = coll
        .update_one(
            bson::doc! { "_id": id },
            bson::doc! { "$set": { "primary": true } },
        )
        .await?;
    if res.matched_count == 0 {
        return Err(AppError::NotFound(format!("asset {id}")));
    }
    coll.update_many(
        bson::doc! { "_id": { "$ne": id } },
        bson::doc! { "$set": { "primary": false } },
    )
    .await?;
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

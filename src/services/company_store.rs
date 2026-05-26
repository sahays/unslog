//! Store for the `companies` collection — target-company packets.
//!
//! Read/write surface:
//! * `get` / `find_or_404` — lookups.
//! * `list_sorted` — newest-first list for the index page.
//! * `insert` — persist a freshly-built company row.
//! * `update_packet` — refresh the research packet (and bump updated_at).
//! * `delete` — drop the row (caller handles question cascade via
//!   `services::questions::delete_for_company`).
//! * `names_by_ids` — bulk-load `_id → name` for the home page recent list.

use std::collections::HashMap;

use futures::TryStreamExt;
use mongodb::options::FindOptions;
use mongodb::Database;

use crate::error::AppError;
use crate::models::company::ResearchPacket;
use crate::models::Company;

pub async fn get(db: &Database, id: &str) -> Result<Option<Company>, AppError> {
    Ok(db
        .collection::<Company>(Company::COLLECTION)
        .find_one(bson::doc! { "_id": id })
        .await?)
}

pub async fn find_or_404(db: &Database, id: &str) -> Result<Company, AppError> {
    get(db, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("company {id}")))
}

/// Projected row for the `/companies` list page. The full
/// `research_packet` ships multi-KB per row and the list view only
/// renders name / role / dates / a "packet ready" badge — so we project
/// to a tight row type and replace the packet with a boolean.
pub struct CompanyListRow {
    pub id: String,
    pub name: String,
    pub role: String,
    pub canonical_role: crate::models::Role,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub has_packet: bool,
}

/// All companies, newest-first. Projected for the list page — no
/// research packet body. Aggregation pipeline so `has_packet` is
/// derived server-side; the packet itself (multi-KB per row) never
/// ships back over the wire.
pub async fn list_sorted(db: &Database) -> Result<Vec<CompanyListRow>, AppError> {
    #[derive(serde::Deserialize)]
    struct Row {
        #[serde(rename = "_id")]
        id: String,
        name: String,
        role: String,
        #[serde(default)]
        canonical_role: Option<crate::models::Role>,
        #[serde(with = "crate::models::datetime_compat::required")]
        created_at: chrono::DateTime<chrono::Utc>,
        #[serde(default)]
        has_packet: bool,
    }
    let pipeline = vec![
        bson::doc! { "$sort": { "created_at": -1 } },
        bson::doc! { "$project": {
            "_id": 1,
            "name": 1,
            "role": 1,
            "canonical_role": 1,
            "created_at": 1,
            "has_packet": { "$cond": [ { "$ifNull": ["$research_packet", false] }, true, false ] },
        } },
    ];
    let cursor = db
        .collection::<bson::Document>(Company::COLLECTION)
        .aggregate(pipeline)
        .await?;
    let docs: Vec<bson::Document> = cursor.try_collect().await?;
    let mut out: Vec<CompanyListRow> = Vec::with_capacity(docs.len());
    for d in docs {
        let r: Row = bson::from_document(d)?;
        out.push(CompanyListRow {
            id: r.id,
            name: r.name,
            role: r.role,
            canonical_role: r
                .canonical_role
                .unwrap_or(crate::models::Role::SolutionsArchitect),
            created_at: r.created_at,
            has_packet: r.has_packet,
        });
    }
    Ok(out)
}

/// All companies, sorted by name. Drives the `/practice` picker.
pub async fn list_by_name(db: &Database) -> Result<Vec<Company>, AppError> {
    let opts = FindOptions::builder()
        .sort(bson::doc! { "name": 1 })
        .build();
    let cursor = db
        .collection::<Company>(Company::COLLECTION)
        .find(bson::doc! {})
        .with_options(opts)
        .await?;
    Ok(cursor.try_collect().await?)
}

pub async fn insert(db: &Database, company: &Company) -> Result<(), AppError> {
    db.collection::<Company>(Company::COLLECTION)
        .insert_one(company)
        .await?;
    Ok(())
}

/// Refresh a company's research packet. Bumps `updated_at` so the index
/// page reflects the change.
pub async fn update_packet(
    db: &Database,
    company_id: &str,
    packet: &ResearchPacket,
) -> Result<(), AppError> {
    db.collection::<Company>(Company::COLLECTION)
        .update_one(
            bson::doc! { "_id": company_id },
            bson::doc! {
                "$set": {
                    "research_packet": bson::to_bson(packet)?,
                    // Match the datetime_compat serializer (RFC 3339 string).
                    "updated_at": chrono::Utc::now().to_rfc3339(),
                }
            },
        )
        .await?;
    Ok(())
}

/// Delete a company row. Callers are responsible for cascading the
/// question rows (via `services::questions::delete_for_company`); we keep
/// that cascade out of this store so the boundary is clean.
pub async fn delete(db: &Database, company_id: &str) -> Result<(), AppError> {
    db.collection::<Company>(Company::COLLECTION)
        .delete_one(bson::doc! { "_id": company_id })
        .await?;
    Ok(())
}

pub async fn count(db: &Database) -> Result<u64, AppError> {
    Ok(db
        .collection::<Company>(Company::COLLECTION)
        .count_documents(bson::doc! {})
        .await?)
}

/// Bulk-load company names for a set of ids. Returns `_id → name`. Used by
/// the home page to render "Company · Role" rows for recent sessions in
/// one query instead of N per row.
pub async fn names_by_ids(
    db: &Database,
    ids: &[&str],
) -> Result<HashMap<String, String>, AppError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let cursor = db
        .collection::<Company>(Company::COLLECTION)
        .find(bson::doc! { "_id": { "$in": ids } })
        .await?;
    let rows: Vec<Company> = cursor.try_collect().await?;
    Ok(rows.into_iter().map(|c| (c.id, c.name)).collect())
}

/// Bulk fetch full company rows for a set of ids in one `$in` query.
/// Used by `/practice` to validate the selected company set without N
/// per-id round-trips. Returns rows in arbitrary order — callers re-key
/// via their own input vector if order matters.
pub async fn find_by_ids(db: &Database, ids: &[String]) -> Result<Vec<Company>, AppError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let cursor = db
        .collection::<Company>(Company::COLLECTION)
        .find(bson::doc! { "_id": { "$in": ids } })
        .await?;
    Ok(cursor.try_collect().await?)
}

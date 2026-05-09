//! Canonical category list — seeded on first run, editable later.

use futures::TryStreamExt;
use mongodb::options::FindOptions;
use mongodb::Database;

use crate::error::AppError;
use crate::models::Category;

/// Twelve canonical behavioral-interview competencies.
/// Index 0–11 maps directly to sort_order so the seed list also drives display order.
const SEED: &[(&str, &str, &str)] = &[
    (
        "ownership",
        "Ownership",
        "Drove an outcome end-to-end; took accountability when things went wrong.",
    ),
    (
        "prioritization",
        "Prioritization",
        "Made hard tradeoffs under constraint; chose what to drop or postpone.",
    ),
    (
        "bias_for_action",
        "Bias for Action",
        "Moved decisively with incomplete information; reversed when wrong.",
    ),
    (
        "dealing_with_ambiguity",
        "Dealing with Ambiguity",
        "Operated without clear direction; defined the problem and a path forward.",
    ),
    (
        "customer_obsession",
        "Customer / User Obsession",
        "Worked backwards from real user needs; pushed back on internal preferences.",
    ),
    (
        "collaboration_influence",
        "Collaboration & Influence",
        "Got cross-team or cross-org work done without formal authority.",
    ),
    (
        "conflict_disagreement",
        "Conflict & Disagreement",
        "Disagreed productively; held a position under pressure or yielded for the right reason.",
    ),
    (
        "communication",
        "Communication",
        "Explained complex ideas clearly to mixed audiences; wrote or spoke to drive a decision.",
    ),
    (
        "highest_standards",
        "Highest Standards",
        "Refused sloppy work; raised the bar on quality, craft, or rigour.",
    ),
    (
        "diving_deep",
        "Diving Deep",
        "Went past the surface; root-caused; got into data or technical details.",
    ),
    (
        "strategic_thinking",
        "Strategic Thinking",
        "Long-term vision, frameworks, second-order consequences.",
    ),
    (
        "failure_learning",
        "Failure & Learning",
        "Recovered from a failure; extracted lessons; applied them later.",
    ),
];

/// On startup, ensure the canonical category list is in place. Insert all 12
/// seed rows only if the collection is empty — never overwrite user edits.
pub async fn seed_defaults(db: &Database) -> Result<(), AppError> {
    let coll = db.collection::<Category>(Category::COLLECTION);
    let count = coll.count_documents(bson::doc! {}).await?;
    if count > 0 {
        return Ok(());
    }
    let now = chrono::Utc::now();
    let docs: Vec<Category> = SEED
        .iter()
        .enumerate()
        .map(|(i, (id, name, desc))| Category {
            id: (*id).to_string(),
            name: (*name).to_string(),
            description: (*desc).to_string(),
            sort_order: i as i32,
            created_at: now,
        })
        .collect();
    coll.insert_many(&docs).await?;
    tracing::info!(count = docs.len(), "seeded canonical categories");
    Ok(())
}

pub async fn list_all(db: &Database) -> Result<Vec<Category>, AppError> {
    let opts = FindOptions::builder()
        .sort(bson::doc! { "sort_order": 1, "name": 1 })
        .build();
    let cursor = db
        .collection::<Category>(Category::COLLECTION)
        .find(bson::doc! {})
        .with_options(opts)
        .await?;
    Ok(cursor.try_collect().await?)
}

pub async fn get(db: &Database, id: &str) -> Result<Option<Category>, AppError> {
    Ok(db
        .collection::<Category>(Category::COLLECTION)
        .find_one(bson::doc! { "_id": id })
        .await?)
}

pub async fn save(db: &Database, cat: &Category) -> Result<(), AppError> {
    db.collection::<Category>(Category::COLLECTION)
        .replace_one(bson::doc! { "_id": &cat.id }, cat)
        .upsert(true)
        .await?;
    Ok(())
}

pub async fn delete(db: &Database, id: &str) -> Result<(), AppError> {
    db.collection::<Category>(Category::COLLECTION)
        .delete_one(bson::doc! { "_id": id })
        .await?;
    Ok(())
}

use serde::{Deserialize, Serialize};

/// A canonical behavioral-interview competency category. Single global pool
/// (not per-company) — Amazon's "Ownership" and Meta's equivalent map to the
/// same row here. Cultural nuance lives in the company's research packet.
///
/// Backed by Postgres `categories` table after Phase A. Serde derives stay
/// for now so any cross-resource serializer that re-emits a Category as part
/// of a larger blob (none today, but the model is `pub`) continues to work.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Category {
    #[serde(rename = "_id")]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub sort_order: i32,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl Category {
    /// Legacy Mongo collection name. Retained for `import_from_mongo`;
    /// remove after one-shot import.
    pub const COLLECTION: &'static str = "categories";
}

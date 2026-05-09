use serde::{Deserialize, Serialize};

/// A canonical behavioral-interview competency category. Single global pool
/// (not per-company) — Amazon's "Ownership" and Meta's equivalent map to the
/// same row here. Cultural nuance lives in the company's research packet.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub const COLLECTION: &'static str = "categories";
}

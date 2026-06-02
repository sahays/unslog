use serde::{Deserialize, Serialize};

/// A canonical behavioral-interview competency category. Single global pool
/// (not per-company) — Amazon's "Ownership" and Meta's equivalent map to the
/// same row here. Cultural nuance lives in the company's research packet.
///
/// Backed by Postgres `categories` table. Serde derives stay so any
/// cross-resource serializer that re-emits a Category as part of a larger
/// blob continues to work.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Category {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub sort_order: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

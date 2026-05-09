use serde::{Deserialize, Serialize};

use crate::models::Role;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchSource {
    pub url: String,
    pub title: String,
    #[serde(default)]
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchPacket {
    pub summary: String,
    pub role_jd: String,
    pub values_signal: String,
    #[serde(default)]
    pub sample_questions: Vec<String>,
    #[serde(default)]
    pub sources: Vec<ResearchSource>,
    pub research_prompt_version_id: String,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub last_refreshed_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Company {
    #[serde(rename = "_id")]
    pub id: String,
    pub name: String,
    /// Company-specific role title — kept verbatim so research/critique prompts
    /// can use the actual phrase the company uses (e.g. "Customer Engineer at
    /// Google", "Applied AI Solutions Architect").
    pub role: String,
    /// Canonical role bucket — drives cross-company question pooling. Required
    /// for new companies; defaults to `SolutionsArchitect` only as a stop-gap
    /// for old data the user said would be deleted.
    #[serde(default = "default_canonical_role")]
    pub canonical_role: Role,
    #[serde(default)]
    pub research_packet: Option<ResearchPacket>,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

fn default_canonical_role() -> Role {
    Role::SolutionsArchitect
}

impl Company {
    pub const COLLECTION: &'static str = "companies";

    pub fn new(name: String, role: String, canonical_role: Role) -> Self {
        let now = chrono::Utc::now();
        Self {
            id: uuid::Uuid::now_v7().to_string(),
            name,
            role,
            canonical_role,
            research_packet: None,
            created_at: now,
            updated_at: now,
        }
    }
}

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
    /// Free-form rundown of the rounds (recruiter screen, behavioral, technical,
    /// onsite, etc.) and what to expect in each. Defaulted so packets written
    /// before this field existed still deserialize.
    #[serde(default)]
    pub interview_process: String,
    #[serde(default)]
    pub sample_questions: Vec<String>,
    #[serde(default)]
    pub sources: Vec<ResearchSource>,
    /// URLs OpenRouter's web plugin actually fetched for this packet — pulled
    /// from `message.annotations` on the response. Distinct from `sources`,
    /// which is what the model *claims* it cited (and could fabricate). Empty
    /// for packets written before this field existed; also empty if the web
    /// plugin didn't run, which the UI surfaces as a "no search" warning.
    #[serde(default)]
    pub fetched_urls: Vec<String>,
    pub research_prompt_version_id: String,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub last_refreshed_at: chrono::DateTime<chrono::Utc>,
}

/// Target company. After Phase A Step 6 the canonical store is Postgres
/// (`companies` table); the importer still deserializes legacy Mongo
/// docs via the `_id` rename and the datetime_compat helpers.
///
/// `research_packet` is JSONB on the Postgres side. `owner_id` and
/// `is_public` were added in migrations 0003 and 0006; old Mongo docs
/// default them so the import path still deserializes cleanly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Company {
    #[serde(rename = "_id")]
    pub id: String,
    /// Per-user; set by the calling handler from `CurrentUser::id`.
    #[serde(default)]
    pub owner_id: String,
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
    #[serde(default)]
    pub is_public: bool,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

fn default_canonical_role() -> Role {
    Role::SolutionsArchitect
}

impl Company {
    /// Legacy Mongo collection name. Still referenced by the one-shot
    /// importer (`services::mongo_import::read`) — removed in the final
    /// sub-phase that drops `mongodb`.
    pub const COLLECTION: &'static str = "companies";

    pub fn new(owner_id: String, name: String, role: String, canonical_role: Role) -> Self {
        let now = chrono::Utc::now();
        Self {
            id: crate::services::id_gen::new(crate::services::id_gen::Kind::Company),
            owner_id,
            name,
            role,
            canonical_role,
            research_packet: None,
            is_public: false,
            created_at: now,
            updated_at: now,
        }
    }
}

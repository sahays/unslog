use serde::{Deserialize, Serialize};

use crate::services::id_gen::{self, Kind};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Summary {
    pub id: String,
    /// Per-user; set by the calling handler from `CurrentUser::id`.
    pub owner_id: String,
    pub session_id: String,
    pub company_id: String,
    #[serde(default)]
    pub narrative: String,
    #[serde(default)]
    pub strengths: Vec<String>,
    #[serde(default)]
    pub recurring_weaknesses: Vec<String>,
    #[serde(default)]
    pub blind_spots: Vec<String>,
    #[serde(default)]
    pub company_fit_signal: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl Summary {
    /// Mint a fresh summary id via the shared minter.
    pub fn new_id() -> String {
        id_gen::new(Kind::Summary)
    }
}

/// Shape returned by the summary LLM call. We persist this into a Summary row
/// after wrapping with id/session/company/created_at metadata.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SummaryPayload {
    #[serde(default)]
    pub narrative: String,
    #[serde(default)]
    pub strengths: Vec<String>,
    #[serde(default)]
    pub recurring_weaknesses: Vec<String>,
    #[serde(default)]
    pub blind_spots: Vec<String>,
    #[serde(default)]
    pub company_fit_signal: String,
}

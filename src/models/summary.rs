use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Summary {
    #[serde(rename = "_id")]
    pub id: String,
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
    #[serde(with = "crate::models::datetime_compat::required")]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl Summary {
    pub const COLLECTION: &'static str = "summaries";
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

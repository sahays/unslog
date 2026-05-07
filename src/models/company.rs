use serde::{Deserialize, Serialize};

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
    pub last_refreshed_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Company {
    #[serde(rename = "_id")]
    pub id: String,
    pub name: String,
    pub role: String,
    #[serde(default)]
    pub research_packet: Option<ResearchPacket>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl Company {
    pub const COLLECTION: &'static str = "companies";

    pub fn new(name: String, role: String) -> Self {
        let now = chrono::Utc::now();
        Self {
            id: uuid::Uuid::now_v7().to_string(),
            name,
            role,
            research_packet: None,
            created_at: now,
            updated_at: now,
        }
    }
}

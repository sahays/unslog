use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuestionSource {
    Uploaded,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Question {
    pub id: String,
    pub text: String,
    pub source: QuestionSource,
    pub added_at: chrono::DateTime<chrono::Utc>,
}

impl Question {
    pub fn new(text: String, source: QuestionSource) -> Self {
        Self {
            id: uuid::Uuid::now_v7().to_string(),
            text,
            source,
            added_at: chrono::Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionBank {
    #[serde(rename = "_id")]
    pub id: String,
    pub company_id: String,
    #[serde(default)]
    pub questions: Vec<Question>,
}

impl QuestionBank {
    pub const COLLECTION: &'static str = "question_banks";

    pub fn empty(company_id: String) -> Self {
        Self {
            id: uuid::Uuid::now_v7().to_string(),
            company_id,
            questions: Vec::new(),
        }
    }
}

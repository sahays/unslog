use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Scores {
    pub specificity: u8,
    pub role_clarity: u8,
    pub star_plus_structure: u8,
    pub pitfalls_avoided: u8,
    pub company_fit: u8,
}

impl Scores {
    pub fn average(&self) -> f32 {
        (self.specificity as f32
            + self.role_clarity as f32
            + self.star_plus_structure as f32
            + self.pitfalls_avoided as f32
            + self.company_fit as f32)
            / 5.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    #[serde(default)]
    pub chapter: String,
    #[serde(default)]
    pub section: String,
    #[serde(default)]
    pub quote: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Critique {
    #[serde(default)]
    pub scores: Scores,
    #[serde(default)]
    pub narrative: String,
    #[serde(default)]
    pub citations: Vec<Citation>,
    #[serde(default)]
    pub improved_vs_prior: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attempt {
    pub attempt_n: u32,
    #[serde(default)]
    pub answer_audio_path: Option<String>,
    pub answer_transcript: String,
    #[serde(default)]
    pub critique: Option<Critique>,
    #[serde(default)]
    pub critique_audio_path: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evaluation {
    #[serde(rename = "_id")]
    pub id: String,
    pub session_id: String,
    pub company_id: String,
    pub question_id: String,
    pub question_text: String,
    #[serde(default)]
    pub attempts: Vec<Attempt>,
}

impl Evaluation {
    pub const COLLECTION: &'static str = "evaluations";

    pub fn new(
        session_id: String,
        company_id: String,
        question_id: String,
        question_text: String,
    ) -> Self {
        Self {
            id: uuid::Uuid::now_v7().to_string(),
            session_id,
            company_id,
            question_id,
            question_text,
            attempts: Vec::new(),
        }
    }
}

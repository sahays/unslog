use serde::{Deserialize, Serialize};

use crate::services::id_gen::{self, Kind};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Scores {
    pub specificity: u8,
    pub role_clarity: u8,
    pub star_plus_structure: u8,
    pub pitfalls_avoided: u8,
    /// `None` when the question has no source-company packet (role-only
    /// questions or companies without a research packet). The critique prompt
    /// is told to omit this axis in that case.
    #[serde(default, deserialize_with = "deserialize_optional_u8")]
    pub company_fit: Option<u8>,
}

/// Be forgiving on input: accept `null`, missing, or `0`-ish values when the
/// model omitted the axis. We treat both null and absent as `None`.
fn deserialize_optional_u8<'de, D>(de: D) -> Result<Option<u8>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    Option::<u8>::deserialize(de)
}

impl Scores {
    /// Mean over the four always-present axes plus company_fit when set.
    /// When company_fit is None (role-only question), the mean is over four.
    pub fn average(&self) -> f32 {
        let mut sum = self.specificity as f32
            + self.role_clarity as f32
            + self.star_plus_structure as f32
            + self.pitfalls_avoided as f32;
        let mut n = 4.0;
        if let Some(f) = self.company_fit {
            sum += f as f32;
            n += 1.0;
        }
        sum / n
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
    #[serde(with = "crate::models::datetime_compat::required")]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evaluation {
    #[serde(rename = "_id")]
    pub id: String,
    /// Master-bound for now via `TEMP_OWNER_ID`; per-user once Phase 1 lands.
    /// Legacy Mongo docs lack this column and default to empty — the
    /// importer backfills before insert.
    #[serde(default)]
    pub owner_id: String,
    pub session_id: String,
    pub company_id: String,
    pub question_id: String,
    pub question_text: String,
    #[serde(default)]
    pub attempts: Vec<Attempt>,
}

impl Evaluation {
    /// Legacy Mongo collection name. Retained for the one-shot importer.
    pub const COLLECTION: &'static str = "evaluations";

    pub fn new(
        owner_id: String,
        session_id: String,
        company_id: String,
        question_id: String,
        question_text: String,
    ) -> Self {
        Self {
            id: id_gen::new(Kind::Evaluation),
            owner_id,
            session_id,
            company_id,
            question_id,
            question_text,
            attempts: Vec::new(),
        }
    }
}

#[cfg(test)]
#[path = "evaluation_tests.rs"]
mod tests;

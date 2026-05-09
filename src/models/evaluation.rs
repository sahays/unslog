use serde::{Deserialize, Serialize};

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

#[cfg(test)]
mod tests {
    use super::*;

    fn scores(s: u8, r: u8, st: u8, p: u8, fit: Option<u8>) -> Scores {
        Scores {
            specificity: s,
            role_clarity: r,
            star_plus_structure: st,
            pitfalls_avoided: p,
            company_fit: fit,
        }
    }

    // ── Scores::average ──────────────────────────────────────────────────

    #[test]
    fn case_avg_without_company_fit() {
        // (1+2+3+4)/4 = 2.5
        assert_eq!(scores(1, 2, 3, 4, None).average(), 2.5);
    }

    #[test]
    fn case_avg_with_company_fit() {
        // (2+2+2+2+5)/5 = 2.6
        assert_eq!(scores(2, 2, 2, 2, Some(5)).average(), 2.6);
    }

    #[test]
    fn case_avg_zero_no_panic() {
        // All zeros — must not divide by zero (n is 4 or 5, never 0).
        assert_eq!(scores(0, 0, 0, 0, None).average(), 0.0);
        assert_eq!(scores(0, 0, 0, 0, Some(0)).average(), 0.0);
    }

    #[test]
    fn case_avg_max() {
        assert_eq!(scores(5, 5, 5, 5, Some(5)).average(), 5.0);
    }

    // ── deserialize_optional_u8 (via Scores serde round-trip) ────────────

    #[test]
    fn case_company_fit_null() {
        let json = r#"{"specificity":1,"role_clarity":1,"star_plus_structure":1,"pitfalls_avoided":1,"company_fit":null}"#;
        let s: Scores = serde_json::from_str(json).expect("parse");
        assert_eq!(s.company_fit, None);
    }

    #[test]
    fn case_company_fit_missing() {
        let json =
            r#"{"specificity":1,"role_clarity":1,"star_plus_structure":1,"pitfalls_avoided":1}"#;
        let s: Scores = serde_json::from_str(json).expect("parse");
        assert_eq!(s.company_fit, None);
    }

    #[test]
    fn case_company_fit_number() {
        let json = r#"{"specificity":1,"role_clarity":1,"star_plus_structure":1,"pitfalls_avoided":1,"company_fit":3}"#;
        let s: Scores = serde_json::from_str(json).expect("parse");
        assert_eq!(s.company_fit, Some(3));
    }

    #[test]
    fn case_company_fit_zero_is_some_zero() {
        // Pin current behavior: a literal 0 deserializes to Some(0), NOT
        // None. Doc-comment on deserialize_optional_u8 claims otherwise; the
        // implementation just delegates to Option::<u8>::deserialize so this
        // is what actually happens.
        let json = r#"{"specificity":1,"role_clarity":1,"star_plus_structure":1,"pitfalls_avoided":1,"company_fit":0}"#;
        let s: Scores = serde_json::from_str(json).expect("parse");
        assert_eq!(s.company_fit, Some(0));
    }
}

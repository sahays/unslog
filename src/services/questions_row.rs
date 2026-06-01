//! Row mapper for the `questions` table.
//!
//! `source` and `role` are TEXT + CHECK on the Postgres side but typed
//! enums on the domain side, and `categories` is JSONB → typed Vec, so
//! we map through a raw row shape and convert in one place. Conversion
//! fails fast on out-of-CHECK values (data corruption / schema drift).

use sqlx::types::Json;

use crate::error::AppError;
use crate::models::{Question, QuestionSource, Role};

/// Columns selected by every full-row read, in the order [`QuestionRow`]
/// expects. Centralized so a schema add only touches one place. Plain
/// column names — the SQL is interpolated into `sqlx::query_as`, not
/// the `query_as!` macro, so an `AS "name: Type"` annotation would
/// become part of the actual returned column name and break `FromRow`.
pub(super) const QUESTION_COLS: &str = r#"id, owner_id, text, source, role,
    categories,
    company_id, added_at"#;

#[derive(sqlx::FromRow)]
pub(super) struct QuestionRow {
    pub id: String,
    pub owner_id: String,
    pub text: String,
    pub source: String,
    pub role: String,
    pub categories: Json<Vec<String>>,
    pub company_id: Option<String>,
    pub added_at: chrono::DateTime<chrono::Utc>,
}

impl QuestionRow {
    pub(super) fn try_into_question(self) -> Result<Question, AppError> {
        let source = parse_source(&self.source)?;
        let role = Role::parse(&self.role).ok_or_else(|| {
            AppError::Other(anyhow::anyhow!(
                "questions.role {:?} not in CHECK set",
                self.role
            ))
        })?;
        Ok(Question {
            id: self.id,
            owner_id: self.owner_id,
            text: self.text,
            source,
            role,
            categories: self.categories.0,
            company_id: self.company_id,
            added_at: self.added_at,
        })
    }
}

pub(super) fn parse_source(s: &str) -> Result<QuestionSource, AppError> {
    match s {
        "uploaded" => Ok(QuestionSource::Uploaded),
        "agent" => Ok(QuestionSource::Agent),
        "pitch" => Ok(QuestionSource::Pitch),
        other => Err(AppError::Other(anyhow::anyhow!(
            "questions.source {other:?} not in CHECK set"
        ))),
    }
}

pub(super) fn source_str(s: QuestionSource) -> &'static str {
    match s {
        QuestionSource::Uploaded => "uploaded",
        QuestionSource::Agent => "agent",
        QuestionSource::Pitch => "pitch",
    }
}

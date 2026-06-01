//! `/companies/:id/questions{,/...}` — paste-bulk add, delete, and edit.

use axum::extract::{Form, Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::error::AppError;
use crate::models::QuestionSource;
use crate::services::{category_store, company_store, questions, text_validation};
use crate::startup::AppState;

/// Per-question text cap. Behavioral questions are typically one or two
/// sentences; 500 chars is plenty and rejects pasted essays / DoS attempts.
const MAX_QUESTION_TEXT: usize = 500;
/// Hard cap on how many questions a single paste-bulk can submit. Stops a
/// 5 MB paste from fanning out into 50_000 LLM categorize calls.
const MAX_BULK_QUESTIONS: usize = 100;

#[derive(Deserialize)]
pub struct AddQuestionsForm {
    pub text: String,
}

pub async fn add_questions(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Form(form): Form<AddQuestionsForm>,
) -> Result<Response, AppError> {
    let lines = parse_bulk_questions(&form.text)?;
    let company = company_store::find_or_404(&state.db, &id).await?;

    let appended_n = lines.len();
    questions::categorize_and_append(
        &state.db,
        &state.pool,
        &*state.openrouter,
        lines,
        QuestionSource::Uploaded,
        company.canonical_role,
        Some(id.clone()),
    )
    .await?;
    tracing::info!(
        event = "company.questions.append",
        company_id = %id,
        source = "uploaded",
        n = appended_n,
        "questions appended",
    );
    Ok(Redirect::to(&format!("/companies/{id}")).into_response())
}

pub async fn delete_question(
    State(state): State<AppState>,
    Path((id, qid)): Path<(String, String)>,
) -> Result<Response, AppError> {
    questions::delete(&state.db, &qid).await?;
    tracing::info!(
        event = "question.delete",
        company_id = %id,
        question_id = %qid,
        "question deleted",
    );
    Ok(Redirect::to(&format!("/companies/{id}")).into_response())
}

#[derive(Deserialize)]
pub struct EditQuestionForm {
    pub text: String,
    pub role: String,
    /// Comma-separated list of category IDs (form-encoded as repeated
    /// "categories" entries from a checkbox group).
    #[serde(default)]
    pub categories: Vec<String>,
}

pub async fn edit_question(
    State(state): State<AppState>,
    Path((id, qid)): Path<(String, String)>,
    Form(form): Form<EditQuestionForm>,
) -> Result<Response, AppError> {
    let text = text_validation::sanitize_short(&form.text, MAX_QUESTION_TEXT, "question text")?;
    let role = crate::models::Role::parse(form.role.trim())
        .ok_or_else(|| AppError::BadRequest(format!("unknown role {}", form.role)))?;

    // Filter categories to known IDs only.
    let canonical = category_store::list_all(&state.pool).await?;
    let valid: std::collections::HashSet<String> = canonical.iter().map(|c| c.id.clone()).collect();
    let categories: Vec<String> = form
        .categories
        .into_iter()
        .map(|c| c.trim().to_string())
        .filter(|c| valid.contains(c))
        .collect();

    questions::update(&state.db, &qid, &text, role, &categories).await?;
    tracing::info!(
        event = "question.edit",
        company_id = %id,
        question_id = %qid,
        role = role.as_str(),
        categories_n = categories.len(),
        "question edited",
    );
    Ok(Redirect::to(&format!("/companies/{id}")).into_response())
}

/// Trust-boundary parser for the bulk-paste textarea. Splits on newlines,
/// drops blank lines, sanitizes each survivor, and enforces the
/// [`MAX_BULK_QUESTIONS`] cap mid-stream so a 5 MB paste stops at the cap
/// without buffering the whole thing. Returns the cleaned questions; the
/// caller is responsible for the downstream categorize + persist.
fn parse_bulk_questions(raw: &str) -> Result<Vec<String>, AppError> {
    let mut lines: Vec<String> = Vec::new();
    for raw_line in raw.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let line = text_validation::sanitize_short(trimmed, MAX_QUESTION_TEXT, "question")?;
        lines.push(line);
        if lines.len() > MAX_BULK_QUESTIONS {
            return Err(AppError::BadRequest(format!(
                "too many questions in one paste — cap is {MAX_BULK_QUESTIONS}"
            )));
        }
    }
    if lines.is_empty() {
        return Err(AppError::BadRequest(
            "paste one or more questions, one per line".into(),
        ));
    }
    Ok(lines)
}

#[cfg(test)]
#[path = "questions_tests.rs"]
mod tests;

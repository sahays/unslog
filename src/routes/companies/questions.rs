//! `/companies/:id/questions{,/...}` — paste-bulk add, delete, and edit.

use axum::extract::{Form, Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::error::AppError;
use crate::models::{Company, QuestionSource};
use crate::services::{category_store, questions};
use crate::startup::AppState;

#[derive(Deserialize)]
pub struct AddQuestionsForm {
    pub text: String,
}

pub async fn add_questions(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Form(form): Form<AddQuestionsForm>,
) -> Result<Response, AppError> {
    let lines: Vec<String> = form
        .text
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if lines.is_empty() {
        return Err(AppError::BadRequest(
            "paste one or more questions, one per line".into(),
        ));
    }
    let company: Company = crate::db::companies(&state.db)
        .find_one(bson::doc! { "_id": &id })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("company {id}")))?;

    let appended_n = lines.len();
    questions::categorize_and_append(
        &state.db,
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
    let text = form.text.trim().to_string();
    if text.is_empty() {
        return Err(AppError::BadRequest("question text is required".into()));
    }
    let role = crate::models::Role::parse(form.role.trim())
        .ok_or_else(|| AppError::BadRequest(format!("unknown role {}", form.role)))?;

    // Filter categories to known IDs only.
    let canonical = category_store::list_all(&state.db).await?;
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

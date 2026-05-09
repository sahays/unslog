use askama::Template;
use axum::extract::{Form, Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use futures::TryStreamExt;
use mongodb::options::FindOptions;
use serde::Deserialize;

use std::collections::HashMap;

use crate::error::AppError;
use crate::models::{Company, Question, QuestionSource, Session, Summary};
use crate::services::{category_store, evaluations, questions, research};
use crate::startup::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/companies", get(list).post(create))
        .route("/companies/:id", get(show))
        .route("/companies/:id/refresh-packet", post(refresh_packet))
        .route("/companies/:id/delete", post(delete))
        .route("/companies/:id/questions", post(add_questions))
        .route(
            "/companies/:id/questions/:qid/delete",
            post(delete_question),
        )
        .route("/companies/:id/questions/:qid/edit", post(edit_question))
}

#[derive(Template)]
#[template(path = "companies/list.html")]
struct ListTemplate {
    companies: Vec<Company>,
    openrouter_configured: bool,
    role_options: Vec<(&'static str, &'static str)>,
}

fn role_options() -> Vec<(&'static str, &'static str)> {
    crate::models::Role::ALL
        .iter()
        .map(|r| (r.as_str(), r.display_name()))
        .collect()
}

async fn list(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let coll = crate::db::companies(&state.db);
    let opts = FindOptions::builder()
        .sort(bson::doc! { "created_at": -1 })
        .build();
    let cursor = coll.find(bson::doc! {}).with_options(opts).await?;
    let companies: Vec<Company> = cursor.try_collect().await?;
    let body = ListTemplate {
        companies,
        openrouter_configured: state.openrouter.configured(),
        role_options: role_options(),
    }
    .render()
    .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(body))
}

#[derive(Deserialize)]
pub struct NewCompanyForm {
    pub name: String,
    pub role: String,
    /// Canonical role bucket — string form like "solutions_architect".
    /// Optional in Phase 1 (defaults to SolutionsArchitect); Phase 4 makes
    /// the form's dropdown render and require it.
    #[serde(default)]
    pub canonical_role: String,
}

async fn create(
    State(state): State<AppState>,
    Form(form): Form<NewCompanyForm>,
) -> Result<Response, AppError> {
    let name = form.name.trim().to_string();
    let role = form.role.trim().to_string();
    if name.is_empty() || role.is_empty() {
        return Err(AppError::BadRequest("name and role are required".into()));
    }
    let canonical_role = crate::models::Role::parse(form.canonical_role.trim())
        .unwrap_or(crate::models::Role::SolutionsArchitect);

    let coll = crate::db::companies(&state.db);
    let mut company = Company::new(name.clone(), role.clone(), canonical_role);

    // Run research synchronously — single user, expected to wait. Failures
    // become a packet-less company; user can hit "refresh packet" to retry.
    let agent_questions = match research::run(&state.openrouter, &state.db, &name, &role).await {
        Ok(packet) => {
            let qs = packet.sample_questions.clone();
            company.research_packet = Some(packet);
            qs
        }
        Err(e) => {
            tracing::warn!(error = %e, name, role, "research agent failed; saving company without packet");
            Vec::new()
        }
    };

    coll.insert_one(&company).await?;
    let agent_questions_n = agent_questions.len();
    questions::categorize_and_append(
        &state.db,
        &state.openrouter,
        agent_questions,
        QuestionSource::Agent,
        canonical_role,
        Some(company.id.clone()),
    )
    .await?;
    tracing::info!(
        event = "company.create",
        company_id = %company.id,
        company_name = %company.name,
        role = %company.role,
        canonical_role = canonical_role.as_str(),
        has_packet = company.research_packet.is_some(),
        agent_questions_n,
        "company created",
    );
    Ok(Redirect::to(&format!("/companies/{}", company.id)).into_response())
}

pub struct SessionRow {
    pub session: Session,
    pub eval_count: u64,
    pub summary: Option<Summary>,
}

#[derive(Template)]
#[template(path = "companies/show.html")]
struct ShowTemplate {
    company: Company,
    questions: Vec<Question>,
    sessions: Vec<SessionRow>,
    canonical_categories: Vec<crate::models::Category>,
    role_options: Vec<(&'static str, &'static str)>,
    edit_qid: Option<String>,
}

#[derive(Deserialize)]
pub struct ShowQuery {
    #[serde(default)]
    pub edit: Option<String>,
}

async fn show(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<ShowQuery>,
) -> Result<Html<String>, AppError> {
    let coll = crate::db::companies(&state.db);
    let company: Company = coll
        .find_one(bson::doc! { "_id": &id })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("company {id}")))?;
    let questions_list = questions::list_for_company(&state.db, &id).await?;

    let opts = FindOptions::builder()
        .sort(bson::doc! { "started_at": -1 })
        .build();
    let sessions_raw: Vec<Session> = state
        .db
        .collection::<Session>(Session::COLLECTION)
        .find(bson::doc! { "company_id": &id })
        .with_options(opts)
        .await?
        .try_collect()
        .await?;

    // Bulk-load eval counts and summaries in two queries instead of 2× per
    // session.
    let session_ids: Vec<&str> = sessions_raw.iter().map(|s| s.id.as_str()).collect();
    let counts = evaluations::counts_by_session(&state.db, &session_ids).await?;
    let summary_by_session: HashMap<String, Summary> = if session_ids.is_empty() {
        HashMap::new()
    } else {
        let summaries: Vec<Summary> = state
            .db
            .collection::<Summary>(Summary::COLLECTION)
            .find(bson::doc! { "session_id": { "$in": &session_ids } })
            .await?
            .try_collect()
            .await?;
        summaries
            .into_iter()
            .map(|s| (s.session_id.clone(), s))
            .collect()
    };

    let sessions: Vec<SessionRow> = sessions_raw
        .into_iter()
        .map(|s| {
            let eval_count = counts.get(&s.id).copied().unwrap_or(0) as u64;
            let summary = summary_by_session.get(&s.id).cloned();
            SessionRow {
                session: s,
                eval_count,
                summary,
            }
        })
        .collect();

    let canonical_categories = category_store::list_all(&state.db).await?;

    let body = ShowTemplate {
        company,
        questions: questions_list,
        sessions,
        canonical_categories,
        role_options: role_options(),
        edit_qid: query.edit,
    }
    .render()
    .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(body))
}

#[derive(Deserialize)]
pub struct AddQuestionsForm {
    pub text: String,
}

async fn add_questions(
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

    questions::categorize_and_append(
        &state.db,
        &state.openrouter,
        lines,
        QuestionSource::Uploaded,
        company.canonical_role,
        Some(id.clone()),
    )
    .await?;
    Ok(Redirect::to(&format!("/companies/{id}")).into_response())
}

async fn delete_question(
    State(state): State<AppState>,
    Path((id, qid)): Path<(String, String)>,
) -> Result<Response, AppError> {
    questions::delete(&state.db, &qid).await?;
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

async fn edit_question(
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

async fn refresh_packet(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let coll = crate::db::companies(&state.db);
    let company: Company = coll
        .find_one(bson::doc! { "_id": &id })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("company {id}")))?;

    let packet = research::run(&state.openrouter, &state.db, &company.name, &company.role).await?;

    // Capture sample questions for tagging before the packet gets moved into BSON.
    let sample_questions = packet.sample_questions.clone();

    coll.update_one(
        bson::doc! { "_id": &id },
        bson::doc! {
            "$set": {
                "research_packet": bson::to_bson(&packet)?,
                // Match the datetime_compat serializer (RFC 3339 string).
                "updated_at": chrono::Utc::now().to_rfc3339(),
            }
        },
    )
    .await?;

    // Append + categorize new sample questions, but skip ones whose text is
    // already saved (so user edits + tags survive a refresh).
    if !sample_questions.is_empty() {
        let existing = questions::list_for_company(&state.db, &id).await?;
        let existing_texts: std::collections::HashSet<String> =
            existing.iter().map(|q| q.text.clone()).collect();
        let new_questions: Vec<String> = sample_questions
            .into_iter()
            .filter(|q| !existing_texts.contains(q))
            .collect();
        questions::categorize_and_append(
            &state.db,
            &state.openrouter,
            new_questions,
            QuestionSource::Agent,
            company.canonical_role,
            Some(id.clone()),
        )
        .await?;
    }

    Ok(Redirect::to(&format!("/companies/{id}")).into_response())
}

async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let coll = crate::db::companies(&state.db);
    coll.delete_one(bson::doc! { "_id": &id }).await?;
    let removed = questions::delete_for_company(&state.db, &id).await?;
    tracing::info!(
        event = "company.delete",
        company_id = %id,
        questions_removed = removed,
        "company deleted, questions cascaded",
    );
    Ok(Redirect::to("/companies").into_response())
}

use askama::Template;
use axum::extract::{Form, Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use futures::TryStreamExt;
use mongodb::options::FindOptions;
use serde::Deserialize;

use crate::error::AppError;
use crate::models::{Company, Evaluation, QuestionBank, QuestionSource, Session, Summary};
use crate::services::{question_bank, research, summary as summary_svc};
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
}

#[derive(Template)]
#[template(path = "companies/list.html")]
struct ListTemplate {
    companies: Vec<Company>,
    openrouter_configured: bool,
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
    }
    .render()
    .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(body))
}

#[derive(Deserialize)]
pub struct NewCompanyForm {
    pub name: String,
    pub role: String,
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

    let coll = crate::db::companies(&state.db);
    let mut company = Company::new(name.clone(), role.clone());

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
    question_bank::ensure_for(&state.db, &company.id).await?;
    let agent_questions_n = agent_questions.len();
    if !agent_questions.is_empty() {
        question_bank::append_questions(
            &state.db,
            &company.id,
            agent_questions,
            QuestionSource::Agent,
        )
        .await?;
    }
    tracing::info!(
        event = "company.create",
        company_id = %company.id,
        company_name = %company.name,
        role = %company.role,
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
    bank: QuestionBank,
    sessions: Vec<SessionRow>,
}

async fn show(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Html<String>, AppError> {
    let coll = crate::db::companies(&state.db);
    let company: Company = coll
        .find_one(bson::doc! { "_id": &id })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("company {id}")))?;
    let bank = question_bank::ensure_for(&state.db, &id).await?;

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

    let mut sessions = Vec::with_capacity(sessions_raw.len());
    for s in sessions_raw {
        let eval_count = state
            .db
            .collection::<Evaluation>(Evaluation::COLLECTION)
            .count_documents(bson::doc! { "session_id": &s.id })
            .await?;
        let summary = summary_svc::for_session(&state.db, &s.id).await?;
        sessions.push(SessionRow {
            session: s,
            eval_count,
            summary,
        });
    }

    let body = ShowTemplate {
        company,
        bank,
        sessions,
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
    question_bank::append_questions(&state.db, &id, lines, QuestionSource::Uploaded).await?;
    Ok(Redirect::to(&format!("/companies/{id}")).into_response())
}

async fn delete_question(
    State(state): State<AppState>,
    Path((id, qid)): Path<(String, String)>,
) -> Result<Response, AppError> {
    question_bank::delete_question(&state.db, &id, &qid).await?;
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

    coll.update_one(
        bson::doc! { "_id": &id },
        bson::doc! {
            "$set": {
                "research_packet": bson::to_bson(&packet)?,
                "updated_at": bson::DateTime::now(),
            }
        },
    )
    .await?;

    Ok(Redirect::to(&format!("/companies/{id}")).into_response())
}

async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let coll = crate::db::companies(&state.db);
    coll.delete_one(bson::doc! { "_id": &id }).await?;
    Ok(Redirect::to("/companies").into_response())
}

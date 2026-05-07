use askama::Template;
use axum::extract::{Form, Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use futures::TryStreamExt;
use mongodb::options::FindOptions;
use serde::Deserialize;

use crate::error::AppError;
use crate::models::Company;
use crate::services::research;
use crate::startup::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/companies", get(list).post(create))
        .route("/companies/:id", get(show))
        .route("/companies/:id/refresh-packet", post(refresh_packet))
        .route("/companies/:id/delete", post(delete))
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
    match research::run(&state.openrouter, &state.db, &name, &role).await {
        Ok(packet) => company.research_packet = Some(packet),
        Err(e) => tracing::warn!(error = %e, name, role, "research agent failed; saving company without packet"),
    }

    coll.insert_one(&company).await?;
    Ok(Redirect::to(&format!("/companies/{}", company.id)).into_response())
}

#[derive(Template)]
#[template(path = "companies/show.html")]
struct ShowTemplate {
    company: Company,
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
    let body = ShowTemplate { company }
        .render()
        .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(body))
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

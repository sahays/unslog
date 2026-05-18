//! `/companies` list, new-form, and create handlers.

use askama::Template;
use axum::extract::{Form, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use futures::TryStreamExt;
use mongodb::options::FindOptions;
use serde::Deserialize;

use crate::error::AppError;
use crate::filters; // Custom Askama filters used by templates below.
use crate::models::Company;
use crate::services::{redact, research, text_validation};
use crate::startup::AppState;

/// Server-side input limits for the new-company form. Mirrored client-side
/// via `maxlength` in `templates/companies/new.html`.
const MAX_COMPANY_NAME: usize = 200;
const MAX_COMPANY_ROLE: usize = 200;

#[derive(Template)]
#[template(path = "companies/list.html")]
struct ListTemplate {
    companies: Vec<Company>,
}

#[derive(Template)]
#[template(path = "companies/new.html")]
struct NewTemplate {
    openrouter_configured: bool,
    role_options: Vec<(&'static str, &'static str)>,
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

pub async fn list(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let coll = crate::db::companies(&state.db);
    let opts = FindOptions::builder()
        .sort(bson::doc! { "created_at": -1 })
        .build();
    let cursor = coll.find(bson::doc! {}).with_options(opts).await?;
    let companies: Vec<Company> = cursor.try_collect().await?;
    let body = ListTemplate { companies }
        .render()
        .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(body))
}

pub async fn new_form(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let body = NewTemplate {
        openrouter_configured: state.openrouter.configured(),
        role_options: super::role_options(),
    }
    .render()
    .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
    Ok(Html(body))
}

pub async fn create(
    State(state): State<AppState>,
    Form(form): Form<NewCompanyForm>,
) -> Result<Response, AppError> {
    let name = text_validation::sanitize_short(&form.name, MAX_COMPANY_NAME, "name")?;
    let role = text_validation::sanitize_short(&form.role, MAX_COMPANY_ROLE, "role")?;
    let canonical_role = crate::models::Role::parse(form.canonical_role.trim())
        .unwrap_or(crate::models::Role::SolutionsArchitect);

    let coll = crate::db::companies(&state.db);
    let mut company = Company::new(name.clone(), role.clone(), canonical_role);

    // Run research synchronously — single user, expected to wait. Failures
    // become a packet-less company; user can hit "refresh packet" to retry.
    let agent_questions = match research::run(&*state.openrouter, &state.db, &name, &role).await {
        Ok(packet) => {
            let qs = packet.sample_questions.clone();
            company.research_packet = Some(packet);
            qs
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                name = %redact::preview(&name, 80),
                role = %redact::preview(&role, 80),
                "research agent failed; saving company without packet",
            );
            Vec::new()
        }
    };

    coll.insert_one(&company).await?;
    let agent_questions_n =
        super::append_agent_questions(&state, &company, agent_questions).await?;
    tracing::info!(
        event = "company.create",
        company_id = %company.id,
        company_name = %redact::preview(&company.name, 80),
        role = %redact::preview(&company.role, 80),
        canonical_role = canonical_role.as_str(),
        has_packet = company.research_packet.is_some(),
        agent_questions_n,
        "company created",
    );
    Ok(Redirect::to(&format!("/companies/{}", company.id)).into_response())
}

//! `/companies/:id/{refresh-packet,delete}` — packet refresh + cascade delete.

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};

use crate::error::AppError;
use crate::services::{
    company_store, questions,
    research::{self, ResearchCtx},
};
use crate::startup::AppState;

pub async fn refresh_packet(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let company = super::load_company(&state, &id).await?;

    let research_ctx = ResearchCtx { pool: &state.pool };
    let packet = research::run(
        &research_ctx,
        &*state.openrouter,
        &company.name,
        &company.role,
    )
    .await?;

    // Capture sample questions for tagging before the packet gets moved into BSON.
    let sample_questions = packet.sample_questions.clone();

    company_store::update_packet(&state.pool, &id, &packet).await?;

    let appended_n = questions::append_skipping_existing(
        &state.pool,
        &*state.openrouter,
        &company,
        sample_questions,
        company.canonical_role,
    )
    .await?;
    tracing::info!(
        event = "company.refresh_packet",
        company_id = %id,
        fetched_urls_n = packet.fetched_urls.len(),
        new_questions_n = appended_n,
        "research packet refreshed",
    );
    Ok(Redirect::to(&format!("/companies/{id}")).into_response())
}

pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    // Postgres FK CASCADE on `questions.company_id` (migration 0001) drops
    // the question rows automatically; the legacy `delete_for_company` call
    // is kept as a no-op wrapper for one transition cycle.
    company_store::delete(&state.pool, &id).await?;
    let removed = questions::delete_for_company(&state.pool, &id).await?;
    tracing::info!(
        event = "company.delete",
        company_id = %id,
        questions_removed = removed,
        "company deleted, questions cascaded",
    );
    Ok(Redirect::to("/companies").into_response())
}

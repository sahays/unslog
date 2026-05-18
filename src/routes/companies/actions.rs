//! `/companies/:id/{refresh-packet,delete}` — packet refresh + cascade delete.

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};

use crate::error::AppError;
use crate::services::{questions, research};
use crate::startup::AppState;

pub async fn refresh_packet(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let coll = crate::db::companies(&state.db);
    let company = super::load_company(&state, &id).await?;

    let packet = research::run(&*state.openrouter, &state.db, &company.name, &company.role).await?;

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

    let appended_n = super::append_agent_questions(&state, &company, sample_questions).await?;
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

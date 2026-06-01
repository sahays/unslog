//! `/categories` — admin CRUD for the canonical category list.

use askama::Template;
use axum::extract::{Form, Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;

use crate::error::AppError;
use crate::models::Category;
use crate::services::category_store;
use crate::startup::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/categories", get(list).post(create))
        .route("/categories/:id/edit", post(edit))
        .route("/categories/:id/delete", post(delete))
}

#[derive(Template)]
#[template(path = "categories/list.html")]
struct ListTemplate {
    categories: Vec<Category>,
}

async fn list(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let categories = category_store::list_all(&state.pool).await?;
    crate::error::render_html(ListTemplate { categories })
}

#[derive(Deserialize)]
pub struct CreateForm {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub sort_order: Option<i32>,
}

async fn create(
    State(state): State<AppState>,
    Form(form): Form<CreateForm>,
) -> Result<Response, AppError> {
    let id = form.id.trim().to_lowercase().replace(' ', "_");
    let name = form.name.trim().to_string();
    let description = form.description.trim().to_string();
    if id.is_empty() || name.is_empty() {
        return Err(AppError::BadRequest("id and name are required".into()));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(AppError::BadRequest(
            "id must be snake_case (lowercase letters, digits, underscores)".into(),
        ));
    }
    if category_store::get(&state.pool, &id).await?.is_some() {
        return Err(AppError::BadRequest(format!(
            "category with id `{id}` already exists"
        )));
    }
    let cat = Category {
        id,
        name,
        description,
        sort_order: form.sort_order.unwrap_or(99),
        created_at: chrono::Utc::now(),
    };
    category_store::save(&state.pool, &cat).await?;
    tracing::info!(
        event = "category.create",
        id = %cat.id,
        name = %cat.name,
        "category created"
    );
    Ok(Redirect::to("/categories").into_response())
}

#[derive(Deserialize)]
pub struct EditForm {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub sort_order: Option<i32>,
}

async fn edit(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Form(form): Form<EditForm>,
) -> Result<Response, AppError> {
    let mut cat = category_store::get(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("category {id}")))?;
    let name = form.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }
    cat.name = name;
    cat.description = form.description.trim().to_string();
    if let Some(order) = form.sort_order {
        cat.sort_order = order;
    }
    category_store::save(&state.pool, &cat).await?;
    tracing::info!(event = "category.edit", id = %cat.id, "category edited");
    Ok(Redirect::to("/categories").into_response())
}

async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    category_store::delete(&state.pool, &id).await?;
    tracing::info!(event = "category.delete", id = %id, "category deleted");
    Ok(Redirect::to("/categories").into_response())
}

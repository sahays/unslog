//! `/browse` company catalog — index + fork.
//!
//! `index` renders every `is_public = TRUE` company across owners,
//! newest first. Each card carries "from `<owner_label>`" attribution
//! plus either a "Use this company" submit button or a "You own this"
//! badge depending on whether the current user is the source's owner.
//!
//! `fork` deep-copies the source into the current user's namespace via
//! `services::company_fork::fork` (one tx; new company + every question
//! row + audit) and redirects to the new company's detail page. Source
//! must be `is_public = TRUE` — anything else surfaces as
//! `AppError::NotFound`, same shape as if the id didn't exist, so a
//! non-public id can't be probed for existence.

use askama::Template;
use axum::extract::{Extension, Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};

use crate::error::AppError;
use crate::filters; // Custom Askama filters used by templates below.
use crate::services::auth::CurrentUser;
use crate::services::company_fork;
use crate::services::sharing::{self, PublicCompanyCard};
use crate::startup::AppState;

#[derive(Template)]
#[template(path = "browse/index.html")]
struct IndexTemplate {
    cards: Vec<PublicCompanyCard>,
    current_user_id: String,
}

pub async fn index(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> Result<Html<String>, AppError> {
    let cards = sharing::list_public_companies(&state.pool).await?;
    crate::error::render_html(IndexTemplate {
        cards,
        current_user_id: current_user.id,
    })
}

pub async fn fork(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let new_id = company_fork::fork(&state.pool, &id, &current_user.id).await?;
    Ok(Redirect::to(&format!("/companies/{new_id}")).into_response())
}

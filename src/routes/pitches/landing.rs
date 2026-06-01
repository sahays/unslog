//! Pitches landing page (`GET /pitches`) — tile grid of canonical
//! intro/narrative questions with per-tile status badges.

use askama::Template;
use axum::extract::{Extension, State};
use axum::response::Html;

use crate::error::AppError;
use crate::filters; // base.html's sidebar footer calls custom filters.
use crate::models::{Pitch, PitchStatus};
use crate::services::auth::CurrentUser;
use crate::services::pitch_store;
use crate::startup::AppState;

#[derive(Template)]
#[template(path = "pitches/index.html")]
struct IndexTemplate {
    tiles: Vec<PitchTile>,
}

pub struct PitchTile {
    pub slug: String,
    pub question_text: String,
    pub blurb: String,
    pub status: PitchStatus,
}

pub(super) async fn index(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> Result<Html<String>, AppError> {
    let pitches = pitch_store::list_all(&state.pool, &current_user.id).await?;
    let tiles: Vec<PitchTile> = pitches.into_iter().map(tile_from).collect();
    crate::error::render_html(IndexTemplate { tiles })
}

fn tile_from(p: Pitch) -> PitchTile {
    PitchTile {
        slug: p.id,
        question_text: p.question_text,
        blurb: p.blurb,
        status: p.status,
    }
}

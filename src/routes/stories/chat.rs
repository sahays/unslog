//! Chat-driven story flow:
//! * `post_turn` — the candidate sends a reply; the coach responds. If the
//!   coach signals lock-in via `<<LOCK_IN>>` we trigger `story_lockin`.
//! * `generate` — explicit "lock it in now" button; calls `story_lockin`.
//! * `continue_chat` — refine kickoff: append a fresh probe and reopen the
//!   chat for vN+1.

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::Form;
use serde::Deserialize;

use crate::error::AppError;
use crate::models::{ChatRole, ChatTurn, StoryStatus};
use crate::services::{
    category_store, chat_lockin, story_lockin, story_refine, story_store, story_version_store,
    text_validation,
};
use crate::startup::AppState;

/// Max chars a candidate may type into one chat turn. Story answers are
/// typically a few sentences to a paragraph; 5000 covers long narratives
/// while still rejecting accidental pastes of unrelated bulk text.
const MAX_CHAT_TURN_CHARS: usize = 5000;

// ── Post turn ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(super) struct TurnForm {
    pub content: String,
}

pub(super) async fn post_turn(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Form(form): Form<TurnForm>,
) -> Result<Response, AppError> {
    let content = text_validation::sanitize_long(&form.content, MAX_CHAT_TURN_CHARS, "message")?;
    let mut story = super::load_story(&state, &id).await?;
    let competency = category_store::get(&state.pool, &story.competency_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("competency {}", story.competency_id)))?;

    let user_turn = ChatTurn {
        role: ChatRole::User,
        content,
        ts: chrono::Utc::now(),
    };
    super::push_turn(&state, &mut story, user_turn).await?;

    // Ask the coach for its next reply. The AI itself decides whether the
    // candidate has agreed to lock in: if so, it ends its reply with the
    // literal `<<LOCK_IN>>` token (contract spelled out in
    // `prompts/story_chat.md`). We strip the token, persist the rest as the
    // final coach turn, and trigger story_lockin.
    let raw = super::run_chat_model(&state, &story, &competency).await?;
    let (cleaned, lock_in) = chat_lockin::strip_lock_in_token(&raw);

    if !cleaned.is_empty() {
        let assistant_turn = ChatTurn {
            role: ChatRole::Assistant,
            content: cleaned,
            ts: chrono::Utc::now(),
        };
        super::push_turn(&state, &mut story, assistant_turn).await?;
    }

    if lock_in {
        story_lockin::generate_and_save(&state.pool, state.openrouter.as_ref(), &story).await?;
    }

    Ok(Redirect::to(&format!("/stories/{id}")).into_response())
}

// ── Generate ─────────────────────────────────────────────────────────────

pub(super) async fn generate(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let story = super::load_story(&state, &id).await?;
    story_lockin::generate_and_save(&state.pool, state.openrouter.as_ref(), &story).await?;
    Ok(Redirect::to(&format!("/stories/{id}")).into_response())
}

// ── Refine kickoff ───────────────────────────────────────────────────────

pub(super) async fn continue_chat(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let mut story = super::load_story(&state, &id).await?;
    let Some(vid) = story.current_version_id.clone() else {
        return Err(AppError::BadRequest(
            "no current version to refine — generate v1 first".into(),
        ));
    };
    let version = story_version_store::get(&state.pool, &vid)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("story version {vid}")))?;

    let probe =
        story_refine::kickoff(&state.pool, state.openrouter.as_ref(), &story, &version).await?;

    let turn = ChatTurn {
        role: ChatRole::Assistant,
        content: probe,
        ts: chrono::Utc::now(),
    };
    super::push_turn(&state, &mut story, turn).await?;

    // Reopen the chat: status returns to InProgress so the next Generate
    // creates vN+1 instead of being a no-op visually.
    story_store::set_status(&state.pool, &story.id, StoryStatus::InProgress).await?;
    tracing::info!(
        event = "story.continue",
        story_id = %story.id,
        from_version = %vid,
        "refine kickoff probe appended"
    );

    Ok(Redirect::to(&format!("/stories/{id}")).into_response())
}

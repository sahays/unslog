//! `/pitches/*` routes — Pitches (intro/narrative answer bank).
//!
//! Pick a pitch question → AI-led probing chat → Lock-in produces spoken
//! short + long prose (no bullets layer). Refine = continue chat → vN+1.
//!
//! Mirrors the Stories module's split:
//! * `landing` — `index` (tile grid).
//! * `show` — show one pitch (chat + composer + side panel) and one
//!   read-only past version.
//! * `chat` — post a turn, lock in, refine.

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;

use crate::error::AppError;
use crate::models::{ChatTurn, Pitch};
use crate::services::openrouter::ChatMessage;
use crate::services::{pitch_store, prompt_escape, resume_context};
use crate::startup::AppState;

mod chat;
mod landing;
mod show;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/pitches", get(landing::index))
        .route("/pitches/:slug", get(show::show))
        .route("/pitches/:slug/turns", post(chat::post_turn))
        .route("/pitches/:slug/generate", post(chat::generate))
        .route("/pitches/:slug/continue", post(chat::continue_chat))
        .route("/pitches/:slug/reset", post(reset_pitch))
        .route("/pitches/:slug/versions/:vid", get(show::show_version))
        .route(
            "/pitches/:slug/versions/:vid/regenerate",
            post(show::regenerate_version),
        )
}

// ── Helpers shared by landing + show + chat ──────────────────────────────

async fn load_pitch(state: &AppState, slug: &str) -> Result<Pitch, AppError> {
    pitch_store::get(&state.db, slug)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("pitch {slug}")))
}

/// Build [system, ...history] and call the chat model. The system prompt
/// is `pitch_chat`; the `<pitch>` block tells the coach which intro
/// question is in play (parallel to the `<competency>` block on Stories).
async fn run_chat_model(state: &AppState, pitch: &Pitch) -> Result<String, AppError> {
    let settings = state.settings_cache.get(&state.db).await?;
    let system_body = state.prompt_cache.get(&state.db, "pitch_chat").await?;
    // Pitch catalog fields are seeded but editable in principle — escape
    // every interpolation so a stray `</pitch>` can't break out and
    // re-direct the coach prompt.
    let pitch_block = format!(
        "<pitch>\nslug: {}\nquestion: {}\nblurb: {}\n</pitch>",
        prompt_escape::for_tag(&pitch.id),
        prompt_escape::for_tag(&pitch.question_text),
        prompt_escape::for_tag(&pitch.blurb),
    );
    // Resume context is optional — empty string when the user hasn't
    // uploaded one. Same join helper as the story coach.
    let resume_block = resume_context::block(state).await?;
    let system = resume_context::join_blocks(&[&system_body, &resume_block, &pitch_block]);
    tracing::info!(
        event = "pitch.chat",
        pitch_id = %pitch.id,
        resume_loaded = !resume_block.is_empty(),
        turns = pitch.chat.len(),
        "pitch chat model invocation",
    );

    let mut messages: Vec<ChatMessage> = Vec::with_capacity(pitch.chat.len() + 2);
    messages.push(ChatMessage::system(system));
    for t in &pitch.chat {
        messages.push(ChatMessage {
            role: t.role.as_str().to_string(),
            content: t.content.clone(),
        });
    }
    // Empty chat → prompt the model to open with one focused question.
    // Same kick as Stories does on first turn.
    if pitch.chat.is_empty() {
        messages.push(ChatMessage::user(
            "Begin the conversation with one focused opening probe for this pitch question.",
        ));
    }

    let reply = state
        .openrouter
        .chat(&settings.critique_model, messages, false)
        .await?;
    let reply = crate::services::llm_safety::check_output("pitch_chat", &reply)?;
    Ok(reply.trim().to_string())
}

// ── Small handlers ───────────────────────────────────────────────────────

/// Reset a pitch: wipe chat + version pointer, return status to not_started.
/// Past versions are kept (immutable history) but no longer current.
async fn reset_pitch(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Response, AppError> {
    pitch_store::reset(&state.db, &slug).await?;
    tracing::info!(event = "pitch.reset", pitch_id = %slug, "pitch reset to not_started");
    Ok(Redirect::to(&format!("/pitches/{slug}")).into_response())
}

// ── Re-exported helpers used inside the module ───────────────────────────

pub(crate) async fn push_turn(
    state: &AppState,
    pitch: &mut Pitch,
    turn: ChatTurn,
) -> Result<(), AppError> {
    pitch_store::push_turn(&state.db, pitch, turn).await
}

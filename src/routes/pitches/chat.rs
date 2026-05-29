//! Chat-driven pitch flow:
//! * `post_turn` — the candidate sends a reply; the coach responds. If the
//!   coach signals lock-in via `<<LOCK_IN>>` we trigger
//!   `pitch_lockin::generate_and_save`.
//! * `generate` — explicit "lock it in now" button.
//! * `continue_chat` — refine kickoff: append a fresh probe and reopen the
//!   chat for vN+1.
//!
//! Mirrors `routes::stories::chat` minus the bullets/summarize step — at
//! lock-in we go straight from chat to spoken short+long prose.

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::Form;
use serde::Deserialize;

use crate::error::AppError;
use crate::models::{ChatRole, ChatTurn};
use crate::services::{chat_lockin, pitch_lockin, pitch_store, text_validation};
use crate::startup::AppState;

/// Max chars a candidate may type into one chat turn. Generous, since pitch
/// answers can run long when the candidate is laying down a hook or a
/// throughline — same ceiling as Stories.
const MAX_CHAT_TURN_CHARS: usize = 5000;

// ── Post turn ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(super) struct TurnForm {
    pub content: String,
}

pub(super) async fn post_turn(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Form(form): Form<TurnForm>,
) -> Result<Response, AppError> {
    let content = text_validation::sanitize_long(&form.content, MAX_CHAT_TURN_CHARS, "message")?;
    let mut pitch = super::load_pitch(&state, &slug).await?;

    let user_turn = ChatTurn {
        role: ChatRole::User,
        content,
        ts: chrono::Utc::now(),
    };
    super::push_turn(&state, &mut pitch, user_turn).await?;

    let raw = super::run_chat_model(&state, &pitch).await?;
    let (cleaned, lock_in) = chat_lockin::strip_lock_in_token(&raw);

    if !cleaned.is_empty() {
        let assistant_turn = ChatTurn {
            role: ChatRole::Assistant,
            content: cleaned,
            ts: chrono::Utc::now(),
        };
        super::push_turn(&state, &mut pitch, assistant_turn).await?;
    }

    if lock_in {
        pitch_lockin::generate_and_save(&state.db, state.openrouter.as_ref(), &pitch).await?;
    }

    Ok(Redirect::to(&format!("/pitches/{slug}")).into_response())
}

// ── Generate ─────────────────────────────────────────────────────────────

/// Explicit lock-in button — bypasses the AI handshake and goes straight to
/// generating the spoken version from whatever's in the chat.
pub(super) async fn generate(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Response, AppError> {
    let pitch = super::load_pitch(&state, &slug).await?;
    if pitch.chat.is_empty() {
        return Err(AppError::BadRequest(
            "no chat content yet — answer at least one probe before generating".into(),
        ));
    }
    pitch_lockin::generate_and_save(&state.db, state.openrouter.as_ref(), &pitch).await?;
    Ok(Redirect::to(&format!("/pitches/{slug}")).into_response())
}

// ── Refine kickoff ───────────────────────────────────────────────────────

/// Reopen the chat for a refined version. Unlike Stories' `continue_chat`
/// — which loads a `story_refine_open` prompt for a fresh probe — pitches
/// simply re-run the coach with status flipped back to InProgress. The
/// coach reads the existing chat (including the just-locked transition)
/// and naturally probes further. No second prompt needed.
pub(super) async fn continue_chat(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Response, AppError> {
    let mut pitch = super::load_pitch(&state, &slug).await?;
    if pitch.current_version_id.is_none() {
        return Err(AppError::BadRequest(
            "no current version to refine — lock in v1 first".into(),
        ));
    }

    pitch_store::reopen(&state.db, &pitch.id).await?;

    let probe = super::run_chat_model(&state, &pitch).await?;
    let turn = ChatTurn {
        role: ChatRole::Assistant,
        content: probe,
        ts: chrono::Utc::now(),
    };
    super::push_turn(&state, &mut pitch, turn).await?;

    tracing::info!(
        event = "pitch.continue",
        pitch_id = %pitch.id,
        "refine kickoff probe appended"
    );

    Ok(Redirect::to(&format!("/pitches/{slug}")).into_response())
}

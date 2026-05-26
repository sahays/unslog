//! `/stories/*` routes — Story Builder.
//!
//! Pick a competency → AI-led probing chat → Generate locks in a STAR+ bullet
//! version. Refine = continue chat → vN+1.
//!
//! Split into:
//! * `landing` — `index` (the tile grid + completed cards) and `create`.
//! * `show` — show one story (chat + composer + side panels) and one
//!   read-only past version.
//! * `chat` — post a turn, generate the bullet version, kick off a refine.
//!
//! Helpers shared between the three sit here.

use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;

use axum_extra::extract::Form;
use axum_htmx::HxRequest;

use crate::error::AppError;
use crate::models::{Category, ChatTurn, Difficulty, Story};
use crate::services::openrouter::ChatMessage;
use crate::services::{prompt_escape, story_store};
use crate::startup::AppState;

mod chat;
mod landing;
mod show;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/stories", get(landing::index).post(landing::create))
        .route("/stories/:id", get(show::show))
        .route("/stories/:id/turns", post(chat::post_turn))
        .route("/stories/:id/generate", post(chat::generate))
        .route("/stories/:id/continue", post(chat::continue_chat))
        .route("/stories/:id/mode", post(set_mode))
        .route("/stories/:id/delete", post(delete_story))
        .route("/stories/:id/versions/:vid", get(show::show_version))
        .route(
            "/stories/:id/versions/:vid/spoken",
            post(show::generate_spoken),
        )
}

// ── Helpers shared by landing + show + chat ──────────────────────────────

/// Thin route-side wrapper around [`story_store::find_or_404`]. Kept so
/// submodules can call `super::load_story(&state, id)` without taking a
/// `&Database` reference everywhere.
async fn load_story(state: &AppState, id: &str) -> Result<Story, AppError> {
    story_store::find_or_404(&state.db, id).await
}

/// Wrapper around [`story_store::push_turn`] — same reasoning as above.
async fn push_turn(state: &AppState, story: &mut Story, turn: ChatTurn) -> Result<(), AppError> {
    story_store::push_turn(&state.db, story, turn).await
}

/// Build [system, ...history] and call the chat model. Picks the system prompt
/// by the story's difficulty mode. Returns the model's next assistant message
/// content.
async fn run_chat_model(
    state: &AppState,
    story: &Story,
    competency: &Category,
) -> Result<String, AppError> {
    let settings = state.settings_cache.get(&state.db).await?;
    let system_body = state
        .prompt_cache
        .get(&state.db, story.mode.prompt_name())
        .await?;
    // Competency catalog rows are user-editable via `/categories`; escape
    // each field so a stray `</competency>` can't break out of the wrapper.
    let competency_block = format!(
        "<competency>\nname: {}\nid: {}\ndescription: {}\n</competency>",
        prompt_escape::for_tag(&competency.name),
        prompt_escape::for_tag(&competency.id),
        prompt_escape::for_tag(&competency.description),
    );
    let system = format!("{system_body}\n\n{competency_block}");

    let mut messages: Vec<ChatMessage> = Vec::with_capacity(story.chat.len() + 2);
    messages.push(ChatMessage::system(system));
    for t in &story.chat {
        messages.push(ChatMessage {
            role: t.role.as_str().to_string(),
            content: t.content.clone(),
        });
    }
    // Most chat models won't generate from a system-only message. When the
    // chat is empty, prepend a one-line user nudge so the assistant produces
    // its opening probe.
    if story.chat.is_empty() {
        messages.push(ChatMessage::user(
            "Begin the conversation with one focused opening probe for this competency.",
        ));
    }

    let reply = state
        .openrouter
        .chat(&settings.critique_model, messages, false)
        .await?;
    let reply = crate::services::llm_safety::check_output("story_chat", &reply)?;
    Ok(reply.trim().to_string())
}

// ── Small handlers ───────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct ModeForm {
    mode: String,
}

/// Update the coach mode for this story. The new mode takes effect on the
/// next chat turn (which loads `story.mode` and picks the matching prompt).
/// Same story, same chat history — only the coach's behavior changes.
#[derive(Template)]
#[template(path = "stories/_mode_pill.html")]
struct ModePillFragment {
    mode: Difficulty,
}

async fn set_mode(
    State(state): State<AppState>,
    HxRequest(is_htmx): HxRequest,
    Path(id): Path<String>,
    Form(form): Form<ModeForm>,
) -> Result<Response, AppError> {
    let mode = Difficulty::from_form(&form.mode);
    story_store::set_mode(&state.db, &id, mode).await?;
    tracing::info!(
        event = "story.set_mode",
        story_id = %id,
        mode = %mode.as_str(),
        "coach mode updated",
    );
    // Htmx callers swap the pill in place; plain form posts (JS off) fall
    // back to a full redirect so the new mode is reflected after reload.
    if is_htmx {
        Ok(crate::error::render_html(ModePillFragment { mode })?.into_response())
    } else {
        Ok(Redirect::to(&format!("/stories/{id}")).into_response())
    }
}

async fn delete_story(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    story_store::delete_cascade(&state.db, &id).await?;
    tracing::info!(event = "story.delete", story_id = %id, "story cascade-deleted");
    Ok(Redirect::to("/stories").into_response())
}

/// Synthesize a placeholder Category when the real row is missing — story.show
/// and story.show_version both want to render *something* rather than 404.
fn unknown_competency(id: &str) -> Category {
    Category {
        id: id.to_string(),
        name: "Unknown competency".to_string(),
        description: String::new(),
        sort_order: 0,
        created_at: chrono::Utc::now(),
    }
}

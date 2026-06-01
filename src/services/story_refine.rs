//! Story refine kickoff — given a locked-in `StoryVersion`, ask the coach
//! for the next probing question. Returns just the probe text; the route
//! handler is responsible for appending the turn and reopening the story
//! (so the boundary between LLM orchestration and persistence stays clean).

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{ChatTurn, Story, StoryVersion};
use crate::services::openrouter::{ChatMessage, LlmClient};
use crate::services::{chat_transcript, llm_safety, prompt_store, settings_store, story_format};

const PROMPT_NAME: &str = "story_refine_open";
const RECENT_CHAT_TURNS: usize = 8;

/// Generate the next coach probe to kick off a refine pass. Returns the
/// probe text (trimmed); the caller appends it as the next chat turn and
/// flips the story status back to `InProgress` so the next Generate
/// produces vN+1 instead of being a no-op.
pub async fn kickoff(
    pool: &PgPool,
    or: &dyn LlmClient,
    story: &Story,
    version: &StoryVersion,
) -> Result<String, AppError> {
    let settings = settings_store::load(pool).await?;
    let system = prompt_store::get_current_body(pool, PROMPT_NAME).await?;

    let recent_str = chat_transcript::render(last_n_in_order(&story.chat, RECENT_CHAT_TURNS));
    let bullets = story_format::bullets(&version.body);
    let user = format!(
        "<current_version>\n{bullets}\n</current_version>\n\n\
         <recent_chat>\n{recent_str}\n</recent_chat>"
    );

    let probe = or
        .chat(
            &settings.critique_model,
            vec![ChatMessage::system(system), ChatMessage::user(user)],
            false,
        )
        .await?;
    let probe = llm_safety::check_output(PROMPT_NAME, &probe)?;
    Ok(probe.trim().to_string())
}

/// Return the last `n` items of `items` in their original order. Cheap
/// alternative to `iter().rev().take(n).collect().into_iter().rev().collect()`.
fn last_n_in_order(items: &[ChatTurn], n: usize) -> &[ChatTurn] {
    &items[items.len().saturating_sub(n)..]
}

#[cfg(test)]
#[path = "story_refine_tests.rs"]
mod tests;

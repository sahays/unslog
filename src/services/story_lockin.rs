//! Story lock-in — turn the coach-candidate chat into a fresh
//! `StoryVersion` (STAR+ bullets), persist it, and repoint the parent
//! `Story` to it. Symmetric to [`crate::services::pitch_lockin`], but with
//! the bullets layer (`StoryBody`) instead of spoken prose.

use mongodb::Database;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{Story, StoryBody, StoryVersion};
use crate::services::openrouter::{self, ChatMessage, LlmClient};
use crate::services::{
    chat_transcript, llm_safety, prompt_store, settings_store, story_store, story_version_store,
};

const PROMPT_NAME: &str = "story_summarize";

/// Run the summarize prompt over the chat, insert the new version, and
/// flip the parent story's `current_version_id` + status. Shared by the
/// explicit Generate button and the `<<LOCK_IN>>`-driven flow in
/// `post_turn`.
pub async fn generate_and_save(
    db: &Database,
    pool: &PgPool,
    or: &dyn LlmClient,
    story: &Story,
) -> Result<StoryVersion, AppError> {
    if story.chat.is_empty() {
        return Err(AppError::BadRequest(
            "no chat content yet — answer at least one probe before generating".into(),
        ));
    }
    let span = tracing::info_span!(
        "story_lockin",
        story_id = %story.id,
        chat_turns = story.chat.len(),
    );
    let _enter = span.enter();
    let start = std::time::Instant::now();

    let body = call_summarize_model(pool, or, story).await?;
    // The unique `(story_id, version_n)` index protects against the
    // double-submit race; the helper retries once on duplicate-key.
    let story_id = story.id.clone();
    let version = story_version_store::insert_with_next_n(db, &story_id, |n| StoryVersion {
        id: uuid::Uuid::now_v7().to_string(),
        story_id: story_id.clone(),
        version_n: n,
        body: body.clone(),
        spoken: None,
        created_at: chrono::Utc::now(),
    })
    .await?;
    story_store::set_current_version(db, &story.id, &version.id).await?;

    tracing::info!(
        event = "story.generate",
        story_id = %story.id,
        version_id = %version.id,
        version_n = version.version_n,
        duration_ms = start.elapsed().as_millis() as u64,
        "story version generated"
    );
    Ok(version)
}

/// One LLM call that returns the `StoryBody` payload. Kept private — the
/// public entry point is [`generate_and_save`].
async fn call_summarize_model(
    pool: &PgPool,
    or: &dyn LlmClient,
    story: &Story,
) -> Result<StoryBody, AppError> {
    let settings = settings_store::load(pool).await?;
    let system = prompt_store::get_current_body_with_schema(pool, PROMPT_NAME).await?;
    let transcript = chat_transcript::render(&story.chat);
    let user = format!("<chat_transcript>\n{transcript}\n</chat_transcript>");

    let raw = or
        .chat(
            &settings.critique_model,
            vec![ChatMessage::system(system), ChatMessage::user(user)],
            true,
        )
        .await?;
    let raw = llm_safety::check_output(PROMPT_NAME, &raw)?;
    openrouter::parse_json_or_log(PROMPT_NAME, &raw)
}

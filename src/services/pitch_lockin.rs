//! Pitch lock-in — single LLM call that turns a coach-candidate chat into
//! `{ short, long }` spoken monologue prose, persisted as a new
//! `PitchVersion`. No bullets layer: intro answers are narrative end-to-end,
//! so the prose IS the artifact (parallel to `story_spoken`, but without
//! the intermediate StoryBody).

use serde::Deserialize;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{Pitch, PitchVersion};
use crate::services::openrouter::{ChatMessage, LlmClient};
use crate::services::{
    chat_transcript, llm_safety, pitch_store, pitch_version_store, prompt_escape, prompt_store,
    settings_store,
};

const PROMPT_NAME: &str = "pitch_lockin";

/// JSON shape `pitch_lockin` returns.
#[derive(Debug, Deserialize)]
struct LockinPayload {
    short: String,
    long: String,
}

/// Lock the current chat into a new `PitchVersion`. Inserts a new
/// version row, repoints `pitch_user_state.current_version_id`, flips
/// status to Locked.
pub async fn generate_and_save(
    pool: &PgPool,
    or: &dyn LlmClient,
    owner_id: &str,
    pitch: &Pitch,
) -> Result<PitchVersion, AppError> {
    let span = tracing::info_span!(
        "pitch_lockin",
        pitch_id = %pitch.id,
        chat_turns = pitch.chat.len(),
    );
    let _enter = span.enter();
    let start = std::time::Instant::now();

    if pitch.chat.is_empty() {
        return Err(AppError::BadRequest(
            "no chat content yet — answer at least one probe before locking in".into(),
        ));
    }

    let payload = call_lockin_model(pool, or, pitch).await?;
    let short = payload.short.trim();
    let long = payload.long.trim();
    // The unique `(pitch_id, version_n)` index protects against the
    // double-submit race; the helper retries once on duplicate-key.
    let version =
        pitch_version_store::insert_with_next_n(pool, owner_id, &pitch.id, short, long).await?;
    pitch_store::set_current_version(pool, owner_id, &pitch.id, &version.id).await?;

    tracing::info!(
        event = "pitch.lockin",
        version_id = %version.id,
        version_n = version.version_n,
        duration_ms = start.elapsed().as_millis() as u64,
        short_words = word_count(&version.short),
        long_words = word_count(&version.long),
        "pitch version locked in",
    );
    Ok(version)
}

/// Re-run the lock-in model and **replace** `version_id` in place (keep
/// the same id and version_n). Used by the regenerate button on the
/// version page when the user wants a different draft from the same chat.
pub async fn regenerate_version(
    pool: &PgPool,
    or: &dyn LlmClient,
    owner_id: &str,
    pitch: &Pitch,
    version_id: &str,
) -> Result<(), AppError> {
    let span = tracing::info_span!(
        "pitch_lockin.regenerate",
        pitch_id = %pitch.id,
        version_id = %version_id,
    );
    let _enter = span.enter();

    if pitch.chat.is_empty() {
        return Err(AppError::BadRequest(
            "no chat content to regenerate from".into(),
        ));
    }

    let payload = call_lockin_model(pool, or, pitch).await?;
    pitch_version_store::replace_in_place(
        pool,
        owner_id,
        version_id,
        &pitch.id,
        payload.short.trim(),
        payload.long.trim(),
    )
    .await?;
    tracing::info!(
        event = "pitch.lockin.regenerated",
        pitch_id = %pitch.id,
        version_id = %version_id,
        "pitch version replaced in place",
    );
    Ok(())
}

async fn call_lockin_model(
    pool: &PgPool,
    or: &dyn LlmClient,
    pitch: &Pitch,
) -> Result<LockinPayload, AppError> {
    let settings = settings_store::load(pool).await?;
    let system = prompt_store::get_current_body_with_schema(pool, PROMPT_NAME).await?;
    let user = render_user_message(pitch);

    let raw = or
        .chat(
            &settings.critique_model,
            vec![ChatMessage::system(system), ChatMessage::user(user)],
            true,
        )
        .await?;
    let raw = llm_safety::check_output(PROMPT_NAME, &raw)?;

    crate::services::openrouter::parse_json_or_log("pitch_lockin", &raw)
}

fn render_user_message(pitch: &Pitch) -> String {
    // Escape every user/catalog-editable field that lands inside the
    // `<pitch>` wrapper so a stray `</pitch>` or `<system>` can't break
    // out and inject directives the model would follow.
    let pitch_block = format!(
        "<pitch>\nslug: {}\nquestion: {}\nblurb: {}\n</pitch>",
        prompt_escape::for_tag(&pitch.id),
        prompt_escape::for_tag(&pitch.question_text),
        prompt_escape::for_tag(&pitch.blurb),
    );
    let transcript = chat_transcript::render(&pitch.chat);
    format!(
        "{pitch_block}\n\n<chat_transcript>\n{transcript}\n</chat_transcript>\n\nWrite the two spoken variants now.",
    )
}

fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}

#[cfg(test)]
#[path = "pitch_lockin_tests.rs"]
mod tests;

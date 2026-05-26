//! Generate spoken-prose variants of a locked-in `StoryVersion`.
//!
//! The candidate's `StoryBody` bullets are the source of truth; this service
//! renders them through the `story_spoken` prompt to produce two
//! verbatim-ready monologues (short ≈3–5 min, long ≈full rehearsal) and
//! persists them onto the same version doc. Calling `generate_and_save`
//! again replaces the previous variants — there's no version history for
//! spoken output (the bullets are the canonical artifact; spoken is a view).

use mongodb::Database;
use serde::Deserialize;

use crate::error::AppError;
use crate::models::{SpokenStory, StoryVersion};
use crate::services::openrouter::{self, ChatMessage, LlmClient};
use crate::services::{llm_safety, prompt_store, settings_store, story_format};

const PROMPT_NAME: &str = "story_spoken";

/// JSON shape the `story_spoken` prompt returns. Mirrors `SpokenStory` minus
/// the server-set `generated_at`.
#[derive(Debug, Deserialize)]
struct SpokenPayload {
    short: String,
    long: String,
}

/// Generate both spoken variants for `version` and persist them onto the
/// version doc. Returns the freshly-built `SpokenStory`. Not idempotent: a
/// rerun replaces whatever was there.
pub async fn generate_and_save(
    db: &Database,
    or: &dyn LlmClient,
    version: &StoryVersion,
) -> Result<SpokenStory, AppError> {
    let span = tracing::info_span!(
        "story_spoken",
        story_id = %version.story_id,
        version_id = %version.id,
        version_n = version.version_n,
    );
    let _enter = span.enter();
    let start = std::time::Instant::now();

    let settings = settings_store::load(db).await?;
    let system = prompt_store::get_current_body_with_schema(db, PROMPT_NAME).await?;
    let user = render_user_message(version);

    let raw = or
        .chat(
            &settings.critique_model,
            vec![ChatMessage::system(system), ChatMessage::user(user)],
            true,
        )
        .await?;
    let raw = llm_safety::check_output(PROMPT_NAME, &raw)?;

    let payload: SpokenPayload = openrouter::parse_json(&raw).map_err(|e| {
        tracing::warn!(
            error = %e,
            raw_preview = %openrouter::preview(&raw, 240),
            "story_spoken JSON parse failed",
        );
        AppError::Upstream(format!(
            "story_spoken returned invalid JSON: {e} — raw: {}",
            openrouter::preview(&raw, 280)
        ))
    })?;

    let spoken = SpokenStory {
        short: payload.short.trim().to_string(),
        long: payload.long.trim().to_string(),
        generated_at: chrono::Utc::now(),
    };

    persist(db, &version.id, &spoken).await?;

    tracing::info!(
        event = "story.spoken.generated",
        duration_ms = start.elapsed().as_millis() as u64,
        short_words = word_count(&spoken.short),
        long_words = word_count(&spoken.long),
        "spoken variants saved",
    );
    Ok(spoken)
}

async fn persist(db: &Database, version_id: &str, spoken: &SpokenStory) -> Result<(), AppError> {
    let doc = bson::to_bson(spoken)?;
    db.collection::<StoryVersion>(StoryVersion::COLLECTION)
        .update_one(
            bson::doc! { "_id": version_id },
            bson::doc! { "$set": { "spoken": doc } },
        )
        .await?;
    Ok(())
}

fn render_user_message(version: &StoryVersion) -> String {
    let bullets = story_format::bullets(&version.body);
    format!("<bullets>\n{bullets}</bullets>\n\nWrite the two spoken variants now.",)
}

/// Whitespace-split word count. Good enough for logging — not for billing.
fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::StoryBody;

    fn body_with_action() -> StoryBody {
        StoryBody {
            situation: vec!["s1".into()],
            task: vec!["t1".into()],
            action: vec!["a1".into(), "a2".into()],
            result: vec![],
            reflection: vec!["r1".into()],
        }
    }

    #[test]
    fn render_user_message_wraps_bullets_and_asks_for_json() {
        let v = StoryVersion {
            id: "v".into(),
            story_id: "s".into(),
            version_n: 1,
            body: body_with_action(),
            spoken: None,
            created_at: chrono::Utc::now(),
        };
        let msg = render_user_message(&v);
        assert!(msg.starts_with("<bullets>\n"), "missing opening tag");
        assert!(msg.contains("Situation:\n- s1"), "missing situation line");
        assert!(msg.contains("Result: (empty)"), "missing empty marker");
        assert!(
            msg.trim_end()
                .ends_with("Write the two spoken variants now."),
            "missing action directive: {msg}"
        );
    }

    #[test]
    fn word_count_handles_multiple_spaces_and_newlines() {
        assert_eq!(word_count(""), 0);
        assert_eq!(word_count("one"), 1);
        assert_eq!(word_count("one   two\nthree\n\nfour"), 4);
    }
}

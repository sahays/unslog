//! Pitch lock-in — single LLM call that turns a coach-candidate chat into
//! `{ short, long }` spoken monologue prose, persisted as a new
//! `PitchVersion`. No bullets layer: intro answers are narrative end-to-end,
//! so the prose IS the artifact (parallel to `story_spoken`, but without
//! the intermediate StoryBody).

use futures::TryStreamExt;
use mongodb::options::FindOptions;
use mongodb::Database;
use serde::Deserialize;

use crate::error::AppError;
use crate::models::{ChatRole, ChatTurn, Pitch, PitchVersion};
use crate::services::openrouter::{self, ChatMessage, LlmClient};
use crate::services::{llm_safety, pitch_store, prompt_store, settings_store};

const PROMPT_NAME: &str = "pitch_lockin";

/// JSON shape `pitch_lockin` returns.
#[derive(Debug, Deserialize)]
struct LockinPayload {
    short: String,
    long: String,
}

/// Lock the current chat into a new `PitchVersion`. Inserts a new
/// version row, repoints `pitches.current_version_id`, flips status to
/// Locked.
pub async fn generate_and_save(
    db: &Database,
    or: &dyn LlmClient,
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

    let payload = call_lockin_model(db, or, pitch).await?;
    let next_n = next_version_n(db, &pitch.id).await?;
    let version = PitchVersion {
        id: uuid::Uuid::now_v7().to_string(),
        pitch_id: pitch.id.clone(),
        version_n: next_n,
        short: payload.short.trim().to_string(),
        long: payload.long.trim().to_string(),
        created_at: chrono::Utc::now(),
    };
    db.collection::<PitchVersion>(PitchVersion::COLLECTION)
        .insert_one(&version)
        .await?;
    pitch_store::set_current_version(db, &pitch.id, &version.id).await?;

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
    db: &Database,
    or: &dyn LlmClient,
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

    let payload = call_lockin_model(db, or, pitch).await?;
    db.collection::<PitchVersion>(PitchVersion::COLLECTION)
        .update_one(
            bson::doc! { "_id": version_id, "pitch_id": &pitch.id },
            bson::doc! { "$set": {
                "short": payload.short.trim(),
                "long": payload.long.trim(),
            } },
        )
        .await?;
    tracing::info!(
        event = "pitch.lockin.regenerated",
        "pitch version replaced in place"
    );
    Ok(())
}

async fn call_lockin_model(
    db: &Database,
    or: &dyn LlmClient,
    pitch: &Pitch,
) -> Result<LockinPayload, AppError> {
    let settings = settings_store::load(db).await?;
    let system = prompt_store::get_current_body_with_schema(db, PROMPT_NAME).await?;
    let user = render_user_message(pitch);

    let raw = or
        .chat(
            &settings.critique_model,
            vec![ChatMessage::system(system), ChatMessage::user(user)],
            true,
        )
        .await?;
    let raw = llm_safety::check_output(PROMPT_NAME, &raw)?;

    openrouter::parse_json(&raw).map_err(|e| {
        tracing::warn!(
            error = %e,
            raw_preview = %openrouter::preview(&raw, 240),
            "pitch_lockin JSON parse failed",
        );
        AppError::Upstream(format!(
            "pitch_lockin returned invalid JSON: {e} — raw: {}",
            openrouter::preview(&raw, 280)
        ))
    })
}

fn render_user_message(pitch: &Pitch) -> String {
    let pitch_block = format!(
        "<pitch>\nslug: {}\nquestion: {}\nblurb: {}\n</pitch>",
        pitch.id, pitch.question_text, pitch.blurb,
    );
    let transcript = render_transcript(&pitch.chat);
    format!(
        "{pitch_block}\n\n<chat_transcript>\n{transcript}\n</chat_transcript>\n\nWrite the two spoken variants now.",
    )
}

fn render_transcript(chat: &[ChatTurn]) -> String {
    chat.iter()
        .map(|t| {
            let label = match t.role {
                ChatRole::User => "CANDIDATE",
                ChatRole::Assistant => "COACH",
            };
            format!("{label}:\n{}", t.content)
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

async fn next_version_n(db: &Database, pitch_id: &str) -> Result<u32, AppError> {
    #[derive(serde::Deserialize)]
    struct VersionNRow {
        version_n: u32,
    }

    let opts = FindOptions::builder()
        .sort(bson::doc! { "version_n": -1 })
        .limit(1)
        .projection(bson::doc! { "version_n": 1 })
        .build();
    let cursor = db
        .collection::<VersionNRow>(PitchVersion::COLLECTION)
        .find(bson::doc! { "pitch_id": pitch_id })
        .with_options(opts)
        .await?;
    let latest: Vec<VersionNRow> = cursor.try_collect().await?;
    Ok(latest.first().map(|v| v.version_n + 1).unwrap_or(1))
}

fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(role: ChatRole, content: &str) -> ChatTurn {
        ChatTurn {
            role,
            content: content.to_string(),
            ts: chrono::Utc::now(),
        }
    }

    fn sample_pitch() -> Pitch {
        Pitch {
            id: "tell-me-about-yourself".into(),
            question_text: "Tell me about yourself.".into(),
            blurb: "blurb".into(),
            sort_order: 0,
            status: crate::models::PitchStatus::InProgress,
            current_version_id: None,
            chat: vec![
                turn(ChatRole::Assistant, "What's your ten-second hook?"),
                turn(
                    ChatRole::User,
                    "I joined the platform team after we paged seventeen times in one week.",
                ),
            ],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn render_user_message_includes_pitch_block_and_transcript_and_action_directive() {
        let msg = render_user_message(&sample_pitch());
        assert!(msg.contains("<pitch>"), "missing pitch open");
        assert!(msg.contains("slug: tell-me-about-yourself"), "missing slug");
        assert!(msg.contains("<chat_transcript>"), "missing transcript open");
        assert!(msg.contains("CANDIDATE:"), "missing candidate label");
        assert!(msg.contains("COACH:"), "missing coach label");
        assert!(
            msg.trim_end()
                .ends_with("Write the two spoken variants now."),
            "missing action directive: {msg}",
        );
    }

    #[test]
    fn render_transcript_separates_turns() {
        let chat = vec![
            turn(ChatRole::Assistant, "ask"),
            turn(ChatRole::User, "answer"),
        ];
        assert_eq!(
            render_transcript(&chat),
            "COACH:\nask\n\n---\n\nCANDIDATE:\nanswer",
        );
    }

    #[test]
    fn word_count_handles_whitespace() {
        assert_eq!(word_count(""), 0);
        assert_eq!(word_count("one   two\nthree\n\nfour"), 4);
    }
}

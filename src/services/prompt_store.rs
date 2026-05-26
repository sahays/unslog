//! Prompt CRUD with append-only versioning.
//!
//! Layout: each prompt lives in `prompts/<name>/prompt.md`, optionally with a
//! sibling `prompts/<name>/schema.json` for prompts that produce structured
//! JSON output. Both files are embedded in the binary via `include_str!`.
//!
//! On first boot, if a `prompts` row doesn't exist for a name, we create the
//! seed `prompt_versions` row from the embedded `prompt.md` and point
//! `prompts.current_version_id` at it. The schema is **not** stored in Mongo
//! — it's a code-coupled file (must match the Rust deserialization struct)
//! and is appended to the prompt body at LLM-request time by
//! [`get_current_body_with_schema`].
//!
//! Saves never overwrite; they always insert a new `prompt_versions` row and
//! flip `prompts.current_version_id`. "Restore version X" is the same path,
//! seeded from an older body.

use mongodb::Database;

use crate::error::AppError;
use crate::models::{Prompt, PromptVersion, PROMPT_NAMES};

const SEED_CRITIQUE: &str = include_str!("../../prompts/critique/prompt.md");
const SEED_RESEARCH: &str = include_str!("../../prompts/research/prompt.md");
const SEED_SUMMARY: &str = include_str!("../../prompts/summary/prompt.md");
const SEED_STORY_CHAT: &str = include_str!("../../prompts/story_chat/prompt.md");
const SEED_STORY_CHAT_COLLAB: &str =
    include_str!("../../prompts/story_chat_collaborative/prompt.md");
const SEED_STORY_SUMMARIZE: &str = include_str!("../../prompts/story_summarize/prompt.md");
const SEED_STORY_REFINE_OPEN: &str = include_str!("../../prompts/story_refine_open/prompt.md");
const SEED_STORY_SPOKEN: &str = include_str!("../../prompts/story_spoken/prompt.md");
const SEED_PITCH_CHAT: &str = include_str!("../../prompts/pitch_chat/prompt.md");
const SEED_PITCH_LOCKIN: &str = include_str!("../../prompts/pitch_lockin/prompt.md");

const SCHEMA_CRITIQUE: &str = include_str!("../../prompts/critique/schema.json");
const SCHEMA_RESEARCH: &str = include_str!("../../prompts/research/schema.json");
const SCHEMA_SUMMARY: &str = include_str!("../../prompts/summary/schema.json");
const SCHEMA_STORY_SUMMARIZE: &str = include_str!("../../prompts/story_summarize/schema.json");
const SCHEMA_STORY_SPOKEN: &str = include_str!("../../prompts/story_spoken/schema.json");
const SCHEMA_PITCH_LOCKIN: &str = include_str!("../../prompts/pitch_lockin/schema.json");

fn seed_for(name: &str) -> Option<&'static str> {
    match name {
        "critique" => Some(SEED_CRITIQUE),
        "research" => Some(SEED_RESEARCH),
        "summary" => Some(SEED_SUMMARY),
        "story_chat" => Some(SEED_STORY_CHAT),
        "story_chat_collaborative" => Some(SEED_STORY_CHAT_COLLAB),
        "story_summarize" => Some(SEED_STORY_SUMMARIZE),
        "story_refine_open" => Some(SEED_STORY_REFINE_OPEN),
        "story_spoken" => Some(SEED_STORY_SPOKEN),
        "pitch_chat" => Some(SEED_PITCH_CHAT),
        "pitch_lockin" => Some(SEED_PITCH_LOCKIN),
        _ => None,
    }
}

/// Output schema for prompts that produce structured JSON. Returns `None`
/// for chat-only prompts (story_chat, story_chat_collaborative, pitch_chat,
/// story_refine_open) — those return prose, not JSON.
pub fn schema_for(name: &str) -> Option<&'static str> {
    match name {
        "critique" => Some(SCHEMA_CRITIQUE),
        "research" => Some(SCHEMA_RESEARCH),
        "summary" => Some(SCHEMA_SUMMARY),
        "story_summarize" => Some(SCHEMA_STORY_SUMMARIZE),
        "story_spoken" => Some(SCHEMA_STORY_SPOKEN),
        "pitch_lockin" => Some(SCHEMA_PITCH_LOCKIN),
        _ => None,
    }
}

/// On startup, ensure each prompt name has a `prompts` row + initial version.
pub async fn seed_defaults(db: &Database) -> Result<(), AppError> {
    let prompts = db.collection::<Prompt>(Prompt::COLLECTION);
    let versions = db.collection::<PromptVersion>(PromptVersion::COLLECTION);

    for name in PROMPT_NAMES {
        let existing = prompts.find_one(bson::doc! { "_id": *name }).await?;
        if existing.is_some() {
            continue;
        }
        let Some(seed) = seed_for(name) else { continue };

        let version = PromptVersion::new((*name).to_string(), seed.to_string(), None);
        versions.insert_one(&version).await?;

        let prompt = Prompt {
            name: (*name).to_string(),
            current_version_id: version.id.clone(),
            updated_at: chrono::Utc::now(),
        };
        prompts.insert_one(&prompt).await?;
        tracing::info!(prompt = name, "seeded default prompt");
    }
    Ok(())
}

/// Save a new version (append-only) and set it current.
pub async fn save_version(
    db: &Database,
    name: &str,
    body: String,
    restored_from: Option<String>,
) -> Result<PromptVersion, AppError> {
    let versions = db.collection::<PromptVersion>(PromptVersion::COLLECTION);
    let prompts = db.collection::<Prompt>(Prompt::COLLECTION);

    let version = PromptVersion::new(name.to_string(), body, restored_from);
    versions.insert_one(&version).await?;

    prompts
        .update_one(
            bson::doc! { "_id": name },
            bson::doc! { "$set": {
                "current_version_id": &version.id,
                "updated_at": chrono::Utc::now().to_rfc3339(),
            } },
        )
        .await?;

    Ok(version)
}

pub async fn get_prompt(db: &Database, name: &str) -> Result<Option<Prompt>, AppError> {
    let prompts = db.collection::<Prompt>(Prompt::COLLECTION);
    Ok(prompts.find_one(bson::doc! { "_id": name }).await?)
}

pub async fn get_version(
    db: &Database,
    version_id: &str,
) -> Result<Option<PromptVersion>, AppError> {
    let versions = db.collection::<PromptVersion>(PromptVersion::COLLECTION);
    Ok(versions.find_one(bson::doc! { "_id": version_id }).await?)
}

pub async fn get_current_body(db: &Database, name: &str) -> Result<String, AppError> {
    let Some(p) = get_prompt(db, name).await? else {
        return Err(AppError::NotFound(format!("prompt {name}")));
    };
    let Some(v) = get_version(db, &p.current_version_id).await? else {
        return Err(AppError::NotFound(format!(
            "prompt version {} for {name}",
            p.current_version_id
        )));
    };
    Ok(v.body)
}

/// Like [`get_current_body`], but appends the output schema (from
/// `prompts/<name>/schema.json`) when one exists for `name`. Use this for
/// the system message of any LLM call that uses `force_json=true` so the
/// model sees the field shape without the prompt body needing to embed it.
/// Chat-only prompts (no schema) are returned unchanged.
pub async fn get_current_body_with_schema(db: &Database, name: &str) -> Result<String, AppError> {
    let body = get_current_body(db, name).await?;
    Ok(with_schema(name, body))
}

/// Like [`get_current_body_with_schema`], but for callers that already hold
/// the body (typically because they resolved a `prompt_snapshot` version
/// id, not the current pointer). Centralizes the append format so the
/// schema block looks identical across every system prompt.
pub fn with_schema(name: &str, body: String) -> String {
    match schema_for(name) {
        Some(schema) => format!("{body}\n\n## Output schema\n\n```\n{}\n```", schema.trim()),
        None => body,
    }
}

pub async fn list_versions(db: &Database, name: &str) -> Result<Vec<PromptVersion>, AppError> {
    use futures::TryStreamExt;
    use mongodb::options::FindOptions;
    let versions = db.collection::<PromptVersion>(PromptVersion::COLLECTION);
    let opts = FindOptions::builder()
        .sort(bson::doc! { "created_at": -1 })
        .build();
    let cursor = versions
        .find(bson::doc! { "prompt_name": name })
        .with_options(opts)
        .await?;
    Ok(cursor.try_collect().await?)
}

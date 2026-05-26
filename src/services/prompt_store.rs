//! Prompt CRUD with append-only versioning.
//!
//! Each prompt lives in `prompts/<name>/prompt.md`, optionally with a
//! sibling `schema.json` for prompts whose output is structured JSON.
//! Both are baked into the binary via `include_str!`.
//!
//! On first boot, missing `prompts` rows get a seed `prompt_versions` row
//! from the embedded `prompt.md`. The schema is **not** stored in Mongo
//! (it's code-coupled — must match the Rust deserialization struct) and
//! is appended at LLM-request time by [`get_current_body_with_schema`].
//!
//! Saves never overwrite; they always insert a new `prompt_versions` row
//! and flip `prompts.current_version_id`. "Restore version X" is the
//! same path, seeded from an older body.

use mongodb::Database;

use crate::error::AppError;
use crate::models::{Prompt, PromptVersion, PROMPT_NAMES};

fn seed_for(name: &str) -> Option<&'static str> {
    match name {
        "critique" => Some(include_str!("../../prompts/critique/prompt.md")),
        "research" => Some(include_str!("../../prompts/research/prompt.md")),
        "summary" => Some(include_str!("../../prompts/summary/prompt.md")),
        "story_chat" => Some(include_str!("../../prompts/story_chat/prompt.md")),
        "story_chat_collaborative" => Some(include_str!(
            "../../prompts/story_chat_collaborative/prompt.md"
        )),
        "story_summarize" => Some(include_str!("../../prompts/story_summarize/prompt.md")),
        "story_refine_open" => Some(include_str!("../../prompts/story_refine_open/prompt.md")),
        "story_spoken" => Some(include_str!("../../prompts/story_spoken/prompt.md")),
        "pitch_chat" => Some(include_str!("../../prompts/pitch_chat/prompt.md")),
        "pitch_lockin" => Some(include_str!("../../prompts/pitch_lockin/prompt.md")),
        _ => None,
    }
}

/// Output schema for prompts that produce structured JSON. Returns `None`
/// for chat-only prompts (story_chat, story_chat_collaborative, pitch_chat,
/// story_refine_open) — those return prose, not JSON.
pub fn schema_for(name: &str) -> Option<&'static str> {
    match name {
        "critique" => Some(include_str!("../../prompts/critique/schema.json")),
        "research" => Some(include_str!("../../prompts/research/schema.json")),
        "summary" => Some(include_str!("../../prompts/summary/schema.json")),
        "story_summarize" => Some(include_str!("../../prompts/story_summarize/schema.json")),
        "story_spoken" => Some(include_str!("../../prompts/story_spoken/schema.json")),
        "pitch_lockin" => Some(include_str!("../../prompts/pitch_lockin/schema.json")),
        _ => None,
    }
}

/// Ensure each prompt name has a `prompts` row + initial version.
///
/// Seed bodies may contain `{{include:_shared/<file>.md}}` markers
/// resolved here ([`resolve_includes`]) **before** writing — stored rows
/// are fully-expanded Markdown, so the `/agents/<name>` edit flow never
/// sees markers. Existing installs (already-seeded rows) are unaffected;
/// the seed only writes when the row is missing.
pub async fn seed_defaults(db: &Database) -> Result<(), AppError> {
    let prompts = db.collection::<Prompt>(Prompt::COLLECTION);
    let versions = db.collection::<PromptVersion>(PromptVersion::COLLECTION);

    for name in PROMPT_NAMES {
        let existing = prompts.find_one(bson::doc! { "_id": *name }).await?;
        if existing.is_some() {
            continue;
        }
        let Some(seed) = seed_for(name) else { continue };
        let body = resolve_includes(seed);

        let version = PromptVersion::new((*name).to_string(), body, None);
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

/// Inline `{{include:_shared/<file>.md}}` markers in a seed body with the
/// matching snippet's static contents. Unknown markers (typos) are left
/// in place so a CI/visual inspection of a seeded prompt catches them.
///
/// Resolution is single-pass — markers inside a snippet body are NOT
/// re-resolved. (Today no snippet references another; the assertion in
/// tests pins that contract.)
pub(crate) fn resolve_includes(body: &str) -> String {
    let mut out = body.to_string();
    for (marker, snippet) in SHARED_SNIPPETS {
        if out.contains(marker) {
            out = out.replace(marker, snippet.trim_end_matches('\n'));
        }
    }
    out
}

/// (marker, snippet body) lookup table for [`resolve_includes`]. Snippet
/// bodies are baked into the binary via `include_str!` so resolution is a
/// pure string replace; never reaches the filesystem at runtime.
pub(crate) const SHARED_SNIPPETS: &[(&str, &str)] = &[
    (
        "{{include:_shared/three_bar_gate.md}}",
        include_str!("../../prompts/_shared/three_bar_gate.md"),
    ),
    (
        "{{include:_shared/three_bar_gate_short.md}}",
        include_str!("../../prompts/_shared/three_bar_gate_short.md"),
    ),
    (
        "{{include:_shared/action_vocab.md}}",
        include_str!("../../prompts/_shared/action_vocab.md"),
    ),
    (
        "{{include:_shared/lock_in_protocol.md}}",
        include_str!("../../prompts/_shared/lock_in_protocol.md"),
    ),
];

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

/// `get_current_body` + `with_schema`. Use for any LLM call that sets
/// `force_json=true` so the model sees the field shape without the body
/// needing to embed it. Chat-only prompts (no schema) are unchanged.
pub async fn get_current_body_with_schema(db: &Database, name: &str) -> Result<String, AppError> {
    let body = get_current_body(db, name).await?;
    Ok(with_schema(name, body))
}

/// Append the named prompt's output schema (if any) to `body`. For callers
/// that already hold a snapshot body and need the schema-appended view.
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

// `PromptCache` lives in `crate::services::prompt_cache`.
// Bulk loaders + list-page aggregation live in `prompt_store_index.rs`.
#[path = "prompt_store_index.rs"]
mod index;
pub use index::{get_all_by_names, list_for_index, ListRow};

#[cfg(test)]
#[path = "prompt_store_tests.rs"]
mod tests;

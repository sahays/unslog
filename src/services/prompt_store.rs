//! Prompt CRUD with append-only versioning.
//!
//! Defaults live in `prompts/<name>.md` and are embedded in the binary via
//! `include_str!`. On first boot, if a `prompts` row doesn't exist for a name,
//! we create the seed `prompt_versions` row from the embedded default and point
//! `prompts.current_version_id` at it.
//!
//! Saves never overwrite; they always insert a new `prompt_versions` row and
//! flip `prompts.current_version_id`. "Restore version X" is the same path,
//! seeded from an older body.

use mongodb::Database;

use crate::error::AppError;
use crate::models::{Prompt, PromptVersion, PROMPT_NAMES};

const SEED_CRITIQUE: &str = include_str!("../../prompts/critique.md");
const SEED_RESEARCH: &str = include_str!("../../prompts/research.md");
const SEED_SUMMARY: &str = include_str!("../../prompts/summary.md");

fn seed_for(name: &str) -> Option<&'static str> {
    match name {
        "critique" => Some(SEED_CRITIQUE),
        "research" => Some(SEED_RESEARCH),
        "summary" => Some(SEED_SUMMARY),
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
            bson::doc! { "$set": { "current_version_id": &version.id, "updated_at": bson::DateTime::now() } },
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

//! Prompt CRUD with append-only versioning.
//!
//! Each prompt lives in `prompts/<name>/prompt.md`, optionally with a
//! sibling `schema.json` for prompts whose output is structured JSON.
//! Both are baked into the binary via `include_str!`.
//!
//! On first boot, missing `prompts` rows get a seed `prompt_versions` row
//! from the embedded `prompt.md`. The schema is **not** stored in Postgres
//! (it's code-coupled — must match the Rust deserialization struct) and
//! is appended at LLM-request time by [`get_current_body_with_schema`].
//!
//! Saves never overwrite; they always insert a new `prompt_versions` row
//! and flip `prompts.current_version_id`. "Restore version X" is the
//! same path, seeded from an older body.

use sqlx::PgPool;

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
pub async fn seed_defaults(pool: &PgPool) -> Result<(), AppError> {
    let existing = list_existing_prompt_names(pool).await?;
    let mut seeded = 0_usize;
    for name in PROMPT_NAMES {
        if existing.contains(*name) {
            continue;
        }
        let Some(seed) = seed_for(name) else { continue };
        let body = resolve_includes(seed);
        seed_one(pool, name, body).await?;
        seeded += 1;
        tracing::info!(
            event = "store.prompts.seed",
            prompt = name,
            "seeded default prompt"
        );
    }
    if seeded > 0 {
        tracing::info!(
            event = "store.prompts.seed.done",
            seeded,
            "prompt seeds applied"
        );
    }
    Ok(())
}

async fn list_existing_prompt_names(
    pool: &PgPool,
) -> Result<std::collections::HashSet<String>, AppError> {
    let rows = sqlx::query!("SELECT name FROM prompts")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| r.name).collect())
}

/// Insert the initial version + prompts row for a single prompt name.
/// Wrapped in a transaction so the FK from prompts.current_version_id to
/// prompt_versions.id is satisfied atomically.
async fn seed_one(pool: &PgPool, name: &str, body: String) -> Result<(), AppError> {
    let version = PromptVersion::new(name.to_string(), body, None);
    let mut tx = pool.begin().await?;
    sqlx::query!(
        r#"
        INSERT INTO prompt_versions (id, prompt_name, body, restored_from)
        VALUES ($1, $2, $3, NULL)
        "#,
        version.id,
        version.prompt_name,
        version.body,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        r#"
        INSERT INTO prompts (name, current_version_id)
        VALUES ($1, $2)
        "#,
        name,
        version.id,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
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

/// Save a new version (append-only) and set it current. Wrapped in a
/// transaction — the prompts pointer must flip atomically with the new
/// version row, or a parallel reader could see the old version after the
/// new one's id is in `prompts.current_version_id`.
pub async fn save_version(
    pool: &PgPool,
    name: &str,
    body: String,
    restored_from: Option<String>,
) -> Result<PromptVersion, AppError> {
    let version = PromptVersion::new(name.to_string(), body, restored_from);
    let mut tx = pool.begin().await?;
    sqlx::query!(
        r#"
        INSERT INTO prompt_versions (id, prompt_name, body, restored_from)
        VALUES ($1, $2, $3, $4)
        "#,
        version.id,
        version.prompt_name,
        version.body,
        version.restored_from,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        r#"
        UPDATE prompts
        SET current_version_id = $2,
            updated_at         = NOW()
        WHERE name = $1
        "#,
        name,
        version.id,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    tracing::info!(
        event = "store.prompts.save_version",
        prompt = name,
        version_id = %version.id,
        "prompt new version persisted",
    );
    Ok(version)
}

pub async fn get_prompt(pool: &PgPool, name: &str) -> Result<Option<Prompt>, AppError> {
    let row = sqlx::query_as!(
        Prompt,
        r#"
        SELECT name, current_version_id, updated_at
        FROM prompts
        WHERE name = $1
        "#,
        name,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn get_version(
    pool: &PgPool,
    version_id: &str,
) -> Result<Option<PromptVersion>, AppError> {
    let row = sqlx::query_as!(
        PromptVersion,
        r#"
        SELECT id, prompt_name, body, created_at, restored_from
        FROM prompt_versions
        WHERE id = $1
        "#,
        version_id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn get_current_body(pool: &PgPool, name: &str) -> Result<String, AppError> {
    let row = sqlx::query!(
        r#"
        SELECT pv.body
        FROM prompts p
        JOIN prompt_versions pv ON pv.id = p.current_version_id
        WHERE p.name = $1
        "#,
        name,
    )
    .fetch_optional(pool)
    .await?;
    row.map(|r| r.body)
        .ok_or_else(|| AppError::NotFound(format!("prompt {name}")))
}

/// `get_current_body` + `with_schema`. Use for any LLM call that sets
/// `force_json=true` so the model sees the field shape without the body
/// needing to embed it. Chat-only prompts (no schema) are unchanged.
pub async fn get_current_body_with_schema(pool: &PgPool, name: &str) -> Result<String, AppError> {
    let body = get_current_body(pool, name).await?;
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

pub async fn list_versions(pool: &PgPool, name: &str) -> Result<Vec<PromptVersion>, AppError> {
    let rows = sqlx::query_as!(
        PromptVersion,
        r#"
        SELECT id, prompt_name, body, created_at, restored_from
        FROM prompt_versions
        WHERE prompt_name = $1
        ORDER BY created_at DESC
        "#,
        name,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

// `PromptCache` lives in `crate::services::prompt_cache`.
// Bulk loaders + list-page aggregation live in `prompt_store_index.rs`.
#[path = "prompt_store_index.rs"]
mod index;
pub use index::{get_all_by_names, list_for_index, ListRow};

#[cfg(test)]
#[path = "prompt_store_tests.rs"]
mod tests;

//! Second half of the write pass: stories, pitches, journal, assets,
//! prompts, settings. Split out of [`super::insert`] purely for the
//! per-file LOC budget.

use anyhow::{Context, Result};
use serde_json::json;
use sqlx::types::Json;
use sqlx::PgPool;

use crate::models::prompt::is_valid_prompt_name;
use crate::models::{
    Asset, JournalEntry, Pitch, PitchVersion, Prompt, PromptVersion, Settings, Story, StoryVersion,
};

use super::id_map::{IdMap, Kind, SETTINGS_SINGLETON_ID};
use super::insert::required_id;

pub async fn insert_stories(
    pool: &PgPool,
    rows: &[Story],
    map: &IdMap,
    owner_id: &str,
) -> Result<()> {
    for s in rows {
        let id = required_id(map, Kind::Story, &s.id, "stories")?;
        let current_version_id = s
            .current_version_id
            .as_deref()
            .and_then(|old| map.get(Kind::StoryVersion, old));
        sqlx::query(
            r#"INSERT INTO stories
               (id, competency_id, status, mode, current_version_id, chat,
                created_at, updated_at, owner_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(&id)
        .bind(&s.competency_id)
        .bind(s.status.as_str())
        .bind(s.mode.as_str())
        .bind(current_version_id)
        .bind(Json(json!(s.chat)))
        .bind(s.created_at)
        .bind(s.updated_at)
        .bind(owner_id)
        .execute(pool)
        .await
        .context("insert stories")?;
    }
    Ok(())
}

pub async fn insert_story_versions(
    pool: &PgPool,
    rows: &[StoryVersion],
    map: &IdMap,
    owner_id: &str,
) -> Result<()> {
    for v in rows {
        let id = required_id(map, Kind::StoryVersion, &v.id, "story_versions")?;
        let story_id = required_id(map, Kind::Story, &v.story_id, "story_versions.story_id")?;
        let spoken = v.spoken.as_ref().map(|s| Json(json!(s)));
        sqlx::query(
            r#"INSERT INTO story_versions
               (id, story_id, version_n, body, spoken, created_at, owner_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               ON CONFLICT (story_id, version_n) DO NOTHING"#,
        )
        .bind(&id)
        .bind(&story_id)
        .bind(v.version_n as i32)
        .bind(Json(json!(v.body)))
        .bind(spoken)
        .bind(v.created_at)
        .bind(owner_id)
        .execute(pool)
        .await
        .context("insert story_versions")?;
    }
    Ok(())
}

pub async fn insert_pitches(pool: &PgPool, rows: &[Pitch]) -> Result<()> {
    for p in rows {
        sqlx::query(
            r#"INSERT INTO pitches (id, question_text, blurb, sort_order, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(&p.id)
        .bind(&p.question_text)
        .bind(&p.blurb)
        .bind(p.sort_order)
        .bind(p.created_at)
        .bind(p.updated_at)
        .execute(pool)
        .await
        .context("insert pitches")?;
    }
    Ok(())
}

pub async fn insert_pitch_versions(
    pool: &PgPool,
    rows: &[PitchVersion],
    map: &IdMap,
    owner_id: &str,
) -> Result<()> {
    // pitch_id is a catalog slug; passes through unchanged.
    for v in rows {
        let id = required_id(map, Kind::PitchVersion, &v.id, "pitch_versions")?;
        sqlx::query(
            r#"INSERT INTO pitch_versions
               (id, pitch_id, version_n, short, long, created_at, owner_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               ON CONFLICT (pitch_id, version_n) DO NOTHING"#,
        )
        .bind(&id)
        .bind(&v.pitch_id)
        .bind(v.version_n as i32)
        .bind(&v.short)
        .bind(&v.long)
        .bind(v.created_at)
        .bind(owner_id)
        .execute(pool)
        .await
        .context("insert pitch_versions")?;
    }
    Ok(())
}

/// Synthesize a per-user pitch state row from the legacy catalog doc.
/// Migration 0004 lifted status/version/chat off the catalog; here we put
/// them back where they now live.
pub async fn insert_pitch_user_state(
    pool: &PgPool,
    rows: &[Pitch],
    map: &IdMap,
    owner_id: &str,
) -> Result<()> {
    for p in rows {
        let current_version_id = p
            .current_version_id
            .as_deref()
            .and_then(|old| map.get(Kind::PitchVersion, old));
        sqlx::query(
            r#"INSERT INTO pitch_user_state
               (owner_id, pitch_id, status, current_version_id, chat, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6)
               ON CONFLICT (owner_id, pitch_id) DO NOTHING"#,
        )
        .bind(owner_id)
        .bind(&p.id)
        .bind(p.status.as_str())
        .bind(current_version_id)
        .bind(Json(json!(p.chat)))
        .bind(p.updated_at)
        .execute(pool)
        .await
        .context("insert pitch_user_state")?;
    }
    Ok(())
}

pub async fn insert_journal_entries(
    pool: &PgPool,
    rows: &[JournalEntry],
    map: &IdMap,
    owner_id: &str,
) -> Result<()> {
    for j in rows {
        let id = required_id(map, Kind::JournalEntry, &j.id, "journal_entries")?;
        sqlx::query(
            r#"INSERT INTO journal_entries
               (id, title, body, created_at, updated_at, archived_at, owner_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(&id)
        .bind(&j.title)
        .bind(&j.body)
        .bind(j.created_at)
        .bind(j.updated_at)
        .bind(j.archived_at)
        .bind(owner_id)
        .execute(pool)
        .await
        .context("insert journal_entries")?;
    }
    Ok(())
}

pub async fn insert_assets(
    pool: &PgPool,
    rows: &[Asset],
    map: &IdMap,
    owner_id: &str,
) -> Result<()> {
    for a in rows {
        let id = required_id(map, Kind::Asset, &a.id, "assets")?;
        sqlx::query(
            r#"INSERT INTO assets (id, name, kind, "primary", original_filename, original_path, extracted_path, extraction_status, extraction_error, uploaded_at, owner_id) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(&id)
        .bind(&a.name)
        .bind(a.kind.as_str())
        .bind(a.primary)
        .bind(&a.original_filename)
        .bind(&a.original_path)
        .bind(&a.extracted_path)
        .bind(match a.extraction_status {
            crate::models::ExtractionStatus::Pending => "pending",
            crate::models::ExtractionStatus::Ok => "ok",
            crate::models::ExtractionStatus::Failed => "failed",
        })
        .bind(&a.extraction_error)
        .bind(a.uploaded_at)
        .bind(owner_id)
        .execute(pool)
        .await
        .context("insert assets")?;
    }
    Ok(())
}

pub async fn insert_prompts(pool: &PgPool, rows: &[Prompt], map: &IdMap) -> Result<()> {
    for p in rows {
        if !is_valid_prompt_name(&p.name) {
            tracing::warn!(
                event = "import.prompt.skipped_legacy",
                name = %p.name,
                "skipping prompt with name not in current PROMPT_NAMES allowlist"
            );
            continue;
        }
        let current_version_id = map
            .get(Kind::PromptVersion, &p.current_version_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "prompts.{}: current_version_id {} not in id map",
                    p.name,
                    p.current_version_id
                )
            })?;
        sqlx::query(
            r#"INSERT INTO prompts (name, current_version_id, updated_at)
               VALUES ($1, $2, $3)
               ON CONFLICT (name) DO NOTHING"#,
        )
        .bind(&p.name)
        .bind(&current_version_id)
        .bind(p.updated_at)
        .execute(pool)
        .await
        .context("insert prompts")?;
    }
    Ok(())
}

pub async fn insert_prompt_versions(
    pool: &PgPool,
    rows: &[PromptVersion],
    map: &IdMap,
) -> Result<()> {
    for v in rows {
        if !is_valid_prompt_name(&v.prompt_name) {
            tracing::warn!(
                event = "import.prompt_version.skipped_legacy",
                prompt_name = %v.prompt_name,
                "skipping prompt_version for legacy prompt name"
            );
            continue;
        }
        let id = required_id(map, Kind::PromptVersion, &v.id, "prompt_versions")?;
        let restored_from = v
            .restored_from
            .as_deref()
            .and_then(|old| map.get(Kind::PromptVersion, old));
        sqlx::query(
            r#"INSERT INTO prompt_versions
               (id, prompt_name, body, created_at, restored_from)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(&id)
        .bind(&v.prompt_name)
        .bind(&v.body)
        .bind(v.created_at)
        .bind(restored_from)
        .execute(pool)
        .await
        .context("insert prompt_versions")?;
    }
    Ok(())
}

pub async fn insert_settings(pool: &PgPool, row: Option<&Settings>) -> Result<()> {
    let Some(s) = row else { return Ok(()) };
    sqlx::query(
        r#"INSERT INTO settings (id, critique_model, research_model, stt_model, tts_model, tts_voice, tts_language, tts_speed, lite_model, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(SETTINGS_SINGLETON_ID)
    .bind(&s.critique_model)
    .bind(&s.research_model)
    .bind(&s.stt_model)
    .bind(&s.tts_model)
    .bind(&s.tts_voice)
    .bind(&s.tts_language)
    .bind(s.tts_speed)
    .bind(&s.lite_model)
    .bind(s.updated_at)
    .execute(pool)
    .await
    .context("insert settings")?;
    Ok(())
}

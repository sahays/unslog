//! Write pass: insert read-pass rows into Postgres with FK rewrites.
//!
//! Every `INSERT` uses `ON CONFLICT (id) DO NOTHING` so re-runs against a
//! partially populated target are a no-op rather than a duplicate-key error.
//! Insert order respects FK dependencies — see the call list in
//! [`insert_all`] for the canonical sequence. Tables for the second half
//! of the dependency chain (stories, pitches, journal, assets, prompts,
//! settings) live in [`super::insert_extras`] to keep each file ≤ 300 LOC.

use anyhow::{Context, Result};
use serde_json::json;
use sqlx::types::Json;
use sqlx::PgPool;

use crate::models::{Category, Company, Evaluation, Question, Session, Summary};

use super::id_map::{IdMap, Kind};
use super::insert_extras;
use super::read::ReadSet;

/// Insert everything in FK-safe order. Logs a structured summary at the
/// end so the operator can sanity-check parity vs. Mongo without dumping
/// row content.
pub async fn insert_all(pool: &PgPool, set: &ReadSet, owner_id: &str) -> Result<()> {
    insert_categories(pool, &set.categories).await?;
    insert_companies(pool, &set.companies, &set.map, owner_id).await?;
    insert_questions(pool, &set.questions, &set.map, owner_id).await?;
    insert_sessions(pool, &set.sessions, &set.map, owner_id).await?;
    insert_evaluations(pool, &set.evaluations, &set.map, owner_id).await?;
    insert_summaries(pool, &set.summaries, &set.map, owner_id).await?;
    insert_extras::insert_stories(pool, &set.stories, &set.map, owner_id).await?;
    insert_extras::insert_story_versions(pool, &set.story_versions, &set.map, owner_id).await?;
    insert_extras::insert_pitches(pool, &set.pitches).await?;
    insert_extras::insert_pitch_versions(pool, &set.pitch_versions, &set.map, owner_id).await?;
    insert_extras::insert_pitch_user_state(pool, &set.pitches, &set.map, owner_id).await?;
    insert_extras::insert_journal_entries(pool, &set.journal_entries, &set.map, owner_id).await?;
    insert_extras::insert_assets(pool, &set.assets, &set.map, owner_id).await?;
    // Prompts MUST be inserted before prompt_versions — the FK is
    // prompt_versions(prompt_name) -> prompts(name). `prompts.current_version_id`
    // has no FK so the forward reference is fine.
    insert_extras::insert_prompts(pool, &set.prompts, &set.map).await?;
    insert_extras::insert_prompt_versions(pool, &set.prompt_versions, &set.map).await?;
    insert_extras::insert_settings(pool, set.settings.as_ref()).await?;
    tracing::info!(event = "import.write.done", "write pass complete");
    Ok(())
}

async fn insert_categories(pool: &PgPool, rows: &[Category]) -> Result<()> {
    for c in rows {
        sqlx::query(
            r#"INSERT INTO categories (id, name, description, sort_order, created_at)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(&c.id)
        .bind(&c.name)
        .bind(&c.description)
        .bind(c.sort_order)
        .bind(c.created_at)
        .execute(pool)
        .await
        .context("insert categories")?;
    }
    Ok(())
}

async fn insert_companies(
    pool: &PgPool,
    rows: &[Company],
    map: &IdMap,
    owner_id: &str,
) -> Result<()> {
    for c in rows {
        let id = required_id(map, Kind::Company, &c.id, "companies")?;
        let packet = c.research_packet.as_ref().map(|p| Json(json!(p)));
        sqlx::query(
            r#"INSERT INTO companies
               (id, name, role, canonical_role, research_packet, created_at, updated_at, owner_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(&id)
        .bind(&c.name)
        .bind(&c.role)
        .bind(c.canonical_role.as_str())
        .bind(packet)
        .bind(c.created_at)
        .bind(c.updated_at)
        .bind(owner_id)
        .execute(pool)
        .await
        .context("insert companies")?;
    }
    Ok(())
}

async fn insert_questions(
    pool: &PgPool,
    rows: &[Question],
    map: &IdMap,
    owner_id: &str,
) -> Result<()> {
    for q in rows {
        let id = required_id(map, Kind::Question, &q.id, "questions")?;
        let company_id = q
            .company_id
            .as_deref()
            .map(|old| required_id(map, Kind::Company, old, "questions.company_id"))
            .transpose()?;
        sqlx::query(
            r#"INSERT INTO questions
               (id, text, source, role, categories, company_id, added_at, owner_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(&id)
        .bind(&q.text)
        .bind(question_source_str(q.source))
        .bind(q.role.as_str())
        .bind(Json(json!(q.categories)))
        .bind(company_id)
        .bind(q.added_at)
        .bind(owner_id)
        .execute(pool)
        .await
        .context("insert questions")?;
    }
    Ok(())
}

async fn insert_sessions(
    pool: &PgPool,
    rows: &[Session],
    map: &IdMap,
    owner_id: &str,
) -> Result<()> {
    for s in rows {
        let id = required_id(map, Kind::Session, &s.id, "sessions")?;
        let company_id = required_id(map, Kind::Company, &s.company_id, "sessions.company_id")?;
        let selected = rewrite_id_list(map, Kind::Company, &s.selected_company_ids);
        let curated = rewrite_id_list(map, Kind::Question, &s.curated_question_ids);
        let current_question_id = s
            .current_question_id
            .as_deref()
            .and_then(|old| map.get(Kind::Question, old));
        sqlx::query(
            r#"INSERT INTO sessions
               (id, company_id, role, selected_company_ids, curated_question_ids,
                focus_line, started_at, ended_at, status, model_snapshot, prompt_snapshot,
                voice_critique_enabled, current_question_id, current_question_text,
                current_question_audio_path, owner_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(&id)
        .bind(&company_id)
        .bind(s.role.as_str())
        .bind(Json(json!(selected)))
        .bind(Json(json!(curated)))
        .bind(&s.focus_line)
        .bind(s.started_at)
        .bind(s.ended_at)
        .bind(s.status.as_str())
        .bind(Json(json!(s.model_snapshot)))
        .bind(Json(json!(s.prompt_snapshot)))
        .bind(s.voice_critique_enabled)
        .bind(current_question_id)
        .bind(&s.current_question_text)
        .bind(&s.current_question_audio_path)
        .bind(owner_id)
        .execute(pool)
        .await
        .context("insert sessions")?;
    }
    Ok(())
}

async fn insert_evaluations(
    pool: &PgPool,
    rows: &[Evaluation],
    map: &IdMap,
    owner_id: &str,
) -> Result<()> {
    for e in rows {
        let id = required_id(map, Kind::Evaluation, &e.id, "evaluations")?;
        let session_id = required_id(map, Kind::Session, &e.session_id, "evaluations.session_id")?;
        let company_id = required_id(map, Kind::Company, &e.company_id, "evaluations.company_id")?;
        let question_id = required_id(
            map,
            Kind::Question,
            &e.question_id,
            "evaluations.question_id",
        )?;
        sqlx::query(
            r#"INSERT INTO evaluations
               (id, session_id, company_id, question_id, question_text, attempts, owner_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(&id)
        .bind(&session_id)
        .bind(&company_id)
        .bind(&question_id)
        .bind(&e.question_text)
        .bind(Json(json!(e.attempts)))
        .bind(owner_id)
        .execute(pool)
        .await
        .context("insert evaluations")?;
    }
    Ok(())
}

async fn insert_summaries(
    pool: &PgPool,
    rows: &[Summary],
    map: &IdMap,
    owner_id: &str,
) -> Result<()> {
    for s in rows {
        let id = required_id(map, Kind::Summary, &s.id, "summaries")?;
        let session_id = required_id(map, Kind::Session, &s.session_id, "summaries.session_id")?;
        let company_id = required_id(map, Kind::Company, &s.company_id, "summaries.company_id")?;
        sqlx::query(
            r#"INSERT INTO summaries
               (id, session_id, company_id, narrative, strengths, recurring_weaknesses,
                blind_spots, company_fit_signal, created_at, owner_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(&id)
        .bind(&session_id)
        .bind(&company_id)
        .bind(&s.narrative)
        .bind(Json(json!(s.strengths)))
        .bind(Json(json!(s.recurring_weaknesses)))
        .bind(Json(json!(s.blind_spots)))
        .bind(&s.company_fit_signal)
        .bind(s.created_at)
        .bind(owner_id)
        .execute(pool)
        .await
        .context("insert summaries")?;
    }
    Ok(())
}

// ── shared helpers ──────────────────────────────────────────────────────

pub(super) fn required_id(map: &IdMap, kind: Kind, old_id: &str, label: &str) -> Result<String> {
    map.get(kind, old_id)
        .ok_or_else(|| anyhow::anyhow!("{label}: id {old_id} (kind {kind:?}) missing from id map"))
}

/// Rewrite each id in a list through the map. Unknown ids are dropped —
/// callers (`sessions.selected_company_ids`, `sessions.curated_question_ids`)
/// can legitimately point at deleted referents in legacy data, and a strict
/// rewrite would reject the whole row.
pub fn rewrite_id_list(map: &IdMap, kind: Kind, old_ids: &[String]) -> Vec<String> {
    old_ids
        .iter()
        .filter_map(|old| map.get(kind, old))
        .collect()
}

fn question_source_str(s: crate::models::QuestionSource) -> &'static str {
    match s {
        crate::models::QuestionSource::Uploaded => "uploaded",
        crate::models::QuestionSource::Agent => "agent",
        crate::models::QuestionSource::Pitch => "pitch",
    }
}

#[cfg(test)]
#[path = "insert_tests.rs"]
mod tests;

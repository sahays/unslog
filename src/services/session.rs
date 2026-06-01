//! Session lifecycle.
//!
//! Owns the snapshot-and-insert that both the single-company entry
//! (`POST /companies/:id/sessions`) and the cross-company entry
//! (`POST /practice`) share. Routes still own validation and the after-insert
//! `advance_to_next` call.

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{ModelSnapshot, PromptSnapshot, Role, Session, SessionStatus, Settings};
use crate::services::current_owner::TEMP_OWNER_ID;
use crate::services::{
    curator::{self, CuratorOutput},
    openrouter::LlmClient,
    prompt_store, sessions, settings_store,
};

pub struct StartInput {
    pub role: Role,
    pub anchor_company_id: String,
    /// Companies the curator should pull questions from. For single-company
    /// sessions this is `[anchor_company_id]`; for `/practice` it can be more.
    pub selected_company_ids: Vec<String>,
}

/// Build a fresh `Session` (snapshotting current models + prompt versions),
/// run the curator to pick questions, and insert. Returns the inserted row.
/// Routes layer their own logging on top.
pub async fn start(
    pool: &PgPool,
    or: &dyn LlmClient,
    input: StartInput,
) -> Result<Session, AppError> {
    let critique_prompt = prompt_store::get_prompt(pool, "critique")
        .await?
        .ok_or_else(|| AppError::NotFound("critique prompt".into()))?;
    let summary_prompt = prompt_store::get_prompt(pool, "summary")
        .await?
        .ok_or_else(|| AppError::NotFound("summary prompt".into()))?;
    let settings = settings_store::load(pool).await?;

    let curated = curator::curate(
        or,
        pool,
        &settings.lite_model,
        input.role,
        &input.selected_company_ids,
    )
    .await?;

    let session = build_session(
        input,
        &settings,
        &critique_prompt.current_version_id,
        &summary_prompt.current_version_id,
        curated,
    );

    sessions::insert(pool, &session).await?;

    Ok(session)
}

/// Assemble a `Session` from already-resolved inputs. Pure — no DB / LLM /
/// clock dependency beyond `Utc::now()` for the started timestamp. Split
/// out so the snapshot-capture invariant can be tested without a database:
/// the critique + summary prompt version IDs MUST land in `prompt_snapshot`
/// verbatim (so subsequent prompt edits cannot retroactively change this
/// session's behavior), and the model fields MUST mirror the current
/// Settings document.
fn build_session(
    input: StartInput,
    settings: &Settings,
    critique_version_id: &str,
    summary_version_id: &str,
    curated: CuratorOutput,
) -> Session {
    Session {
        id: Session::new_id(),
        // TODO(phase-1): replace with `current_user.id`.
        owner_id: TEMP_OWNER_ID.to_string(),
        company_id: input.anchor_company_id,
        role: input.role,
        selected_company_ids: input.selected_company_ids,
        curated_question_ids: curated.question_ids,
        focus_line: curated.focus_line,
        started_at: chrono::Utc::now(),
        ended_at: None,
        status: SessionStatus::Active,
        model_snapshot: ModelSnapshot {
            stt: settings.stt_model.clone(),
            tts: settings.tts_model.clone(),
            critique: settings.critique_model.clone(),
            research: settings.research_model.clone(),
            tts_voice: settings.tts_voice.clone(),
            tts_language: settings.tts_language.clone(),
            tts_speed: settings.tts_speed,
            lite: settings.lite_model.clone(),
        },
        prompt_snapshot: PromptSnapshot {
            critique: critique_version_id.to_string(),
            summary: summary_version_id.to_string(),
        },
        voice_critique_enabled: false,
        current_question_id: None,
        current_question_text: None,
        current_question_audio_path: None,
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;

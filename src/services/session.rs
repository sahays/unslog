//! Session lifecycle.
//!
//! Owns the snapshot-and-insert that both the single-company entry
//! (`POST /companies/:id/sessions`) and the cross-company entry
//! (`POST /practice`) share. Routes still own validation and the after-insert
//! `advance_to_next` call.

use mongodb::Database;

use crate::error::AppError;
use crate::models::{ModelSnapshot, PromptSnapshot, Role, Session, SessionStatus};
use crate::services::{curator, openrouter::LlmClient, prompt_store, settings_store};

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
    db: &Database,
    or: &dyn LlmClient,
    input: StartInput,
) -> Result<Session, AppError> {
    let critique_prompt = prompt_store::get_prompt(db, "critique")
        .await?
        .ok_or_else(|| AppError::NotFound("critique prompt".into()))?;
    let summary_prompt = prompt_store::get_prompt(db, "summary")
        .await?
        .ok_or_else(|| AppError::NotFound("summary prompt".into()))?;
    let settings = settings_store::load(db).await?;

    let curated = curator::curate(
        or,
        db,
        &settings.lite_model,
        input.role,
        &input.selected_company_ids,
    )
    .await?;

    let session = Session {
        id: uuid::Uuid::now_v7().to_string(),
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
            critique: critique_prompt.current_version_id,
            summary: summary_prompt.current_version_id,
        },
        voice_critique_enabled: false,
        current_question_id: None,
        current_question_text: None,
        current_question_audio_path: None,
    };

    db.collection::<Session>(Session::COLLECTION)
        .insert_one(&session)
        .await?;

    Ok(session)
}

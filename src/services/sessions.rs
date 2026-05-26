//! Store for the `sessions` collection — practice-session shells.
//!
//! Read/write surface:
//! * `get` / `find_or_404` — lookups.
//! * `set_current_question` / `clear_current_question` — flips that the
//!   advance/next handlers used to inline.
//! * `end` — flip status to `ended`, stamp `ended_at`, clear current question.
//! * `delete_cascade` — wipe session + evaluations + summary in one place.
//! * `list_active` / `list_for_company` — projected reads for the practice
//!   page and the company show page.
//! * `count_active` / `list_recent` — index-page summaries.
//! * `toggle_voice` — small flip used by the voice toggle handler.

use futures::TryStreamExt;
use mongodb::options::FindOptions;
use mongodb::Database;

use crate::error::AppError;
use crate::models::{Evaluation, Session, SessionStatus, Summary};

pub async fn get(db: &Database, id: &str) -> Result<Option<Session>, AppError> {
    Ok(db
        .collection::<Session>(Session::COLLECTION)
        .find_one(bson::doc! { "_id": id })
        .await?)
}

pub async fn find_or_404(db: &Database, id: &str) -> Result<Session, AppError> {
    get(db, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("session {id}")))
}

/// Persist the next curated question on the active session (id, text,
/// optional audio path). Used by `advance_to_next`.
pub async fn set_current_question(
    db: &Database,
    session_id: &str,
    qid: &str,
    qtext: &str,
    audio_path: Option<&str>,
) -> Result<(), AppError> {
    db.collection::<Session>(Session::COLLECTION)
        .update_one(
            bson::doc! { "_id": session_id },
            bson::doc! { "$set": {
                "current_question_id": qid,
                "current_question_text": qtext,
                "current_question_audio_path": audio_path,
            } },
        )
        .await?;
    Ok(())
}

/// Flip status to `ended`, stamp `ended_at`, and clear current_question_*.
/// Idempotent: writing `ended` to an already-ended session is a no-op
/// against the projection callers care about.
pub async fn end(db: &Database, session_id: &str) -> Result<(), AppError> {
    db.collection::<Session>(Session::COLLECTION)
        .update_one(
            bson::doc! { "_id": session_id },
            bson::doc! { "$set": {
                "status": SessionStatus::Ended.as_str(),
                "ended_at": chrono::Utc::now().to_rfc3339(),
                "current_question_id": null,
                "current_question_text": null,
            } },
        )
        .await?;
    Ok(())
}

/// Flip the `voice_critique_enabled` flag. Returns the new value.
pub async fn toggle_voice(db: &Database, session: &Session) -> Result<bool, AppError> {
    let new_value = !session.voice_critique_enabled;
    db.collection::<Session>(Session::COLLECTION)
        .update_one(
            bson::doc! { "_id": &session.id },
            bson::doc! { "$set": { "voice_critique_enabled": new_value } },
        )
        .await?;
    Ok(new_value)
}

/// Delete a session and every child row it owns (evaluations, summary).
/// Returns `(session_deleted, evals_deleted, summaries_deleted)` so the
/// caller can log the counts.
pub async fn delete_cascade(db: &Database, session_id: &str) -> Result<(u64, u64, u64), AppError> {
    let evals_deleted = db
        .collection::<Evaluation>(Evaluation::COLLECTION)
        .delete_many(bson::doc! { "session_id": session_id })
        .await?
        .deleted_count;
    let summaries_deleted = db
        .collection::<Summary>(Summary::COLLECTION)
        .delete_many(bson::doc! { "session_id": session_id })
        .await?
        .deleted_count;
    let session_deleted = db
        .collection::<Session>(Session::COLLECTION)
        .delete_one(bson::doc! { "_id": session_id })
        .await?
        .deleted_count;
    Ok((session_deleted, evals_deleted, summaries_deleted))
}

/// Active sessions, newest-first. Used by `/practice` to render the
/// in-progress rail.
pub async fn list_active(db: &Database) -> Result<Vec<Session>, AppError> {
    let opts = FindOptions::builder()
        .sort(bson::doc! { "started_at": -1 })
        .build();
    let cursor = db
        .collection::<Session>(Session::COLLECTION)
        .find(bson::doc! { "status": SessionStatus::Active.as_str() })
        .with_options(opts)
        .await?;
    Ok(cursor.try_collect().await?)
}

/// Sessions for one company, newest-first. Used by the company show page.
pub async fn list_for_company(db: &Database, company_id: &str) -> Result<Vec<Session>, AppError> {
    let opts = FindOptions::builder()
        .sort(bson::doc! { "started_at": -1 })
        .build();
    let cursor = db
        .collection::<Session>(Session::COLLECTION)
        .find(bson::doc! { "company_id": company_id })
        .with_options(opts)
        .await?;
    Ok(cursor.try_collect().await?)
}

/// Cheap count of active sessions — for the home page status card.
pub async fn count_active(db: &Database) -> Result<u64, AppError> {
    Ok(db
        .collection::<Session>(Session::COLLECTION)
        .count_documents(bson::doc! { "status": SessionStatus::Active.as_str() })
        .await?)
}

/// Most-recent N sessions, regardless of status. Used by the home page.
pub async fn list_recent(db: &Database, limit: i64) -> Result<Vec<Session>, AppError> {
    let opts = FindOptions::builder()
        .sort(bson::doc! { "started_at": -1 })
        .limit(limit)
        .build();
    let cursor = db
        .collection::<Session>(Session::COLLECTION)
        .find(bson::doc! {})
        .with_options(opts)
        .await?;
    Ok(cursor.try_collect().await?)
}

/// Insert a freshly-built session row.
pub async fn insert(db: &Database, session: &Session) -> Result<(), AppError> {
    db.collection::<Session>(Session::COLLECTION)
        .insert_one(session)
        .await?;
    Ok(())
}

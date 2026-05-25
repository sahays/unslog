//! Canonical pitch catalog — seeded on first run, state preserved across
//! restarts.
//!
//! Each seed row inserts only when the row is missing (`$setOnInsert`), so a
//! deployed app gets new pitch types when the seed list grows without
//! clobbering any chat the user has already accumulated on existing rows.

use futures::TryStreamExt;
use mongodb::options::FindOptions;
use mongodb::Database;

use crate::error::AppError;
use crate::models::{ChatTurn, Pitch, PitchStatus, Question, Role};

/// Seven canonical intro/narrative questions. Slug = `_id`. Tuple is
/// `(slug, question_text, blurb)`. Index drives `sort_order`, so the order
/// here is the order shown on the tile grid.
const SEED: &[(&str, &str, &str)] = &[
    (
        "tell-me-about-yourself",
        "Tell me about yourself.",
        "The opening pitch — who you are, how you got here, why this matters now.",
    ),
    (
        "why-this-role",
        "Why this role?",
        "What about this role fits where you're going next, beyond the company.",
    ),
    (
        "why-this-company",
        "Why this company?",
        "What pulled you specifically here, in this candidate's voice — not the careers page.",
    ),
    (
        "walk-through-resume",
        "Walk me through your resume.",
        "The narrative arc of your career — why each move, what the throughline is.",
    ),
    (
        "five-year-plan",
        "Where do you see yourself in five years?",
        "Direction, not destination. What you want to be doing and learning.",
    ),
    (
        "key-strength",
        "What is your greatest strength?",
        "A real strength with one concrete moment that proves it, not a buzzword.",
    ),
    (
        "key-weakness",
        "What is your greatest weakness?",
        "A real weakness — what you've done about it, what's still in progress.",
    ),
];

/// On startup, idempotently upsert each canonical pitch row and a matching
/// role-only Question for the bank. Existing user state (status, chat,
/// current_version_id, question category tags) is preserved via
/// `$setOnInsert` — only catalog fields are written when a row is missing.
pub async fn seed_defaults(db: &Database) -> Result<(), AppError> {
    let coll = db.collection::<Pitch>(Pitch::COLLECTION);
    let questions = db.collection::<Question>(Question::COLLECTION);
    let now = chrono::Utc::now();
    let now_str = now.to_rfc3339();
    for (i, (slug, question_text, blurb)) in SEED.iter().enumerate() {
        coll.update_one(
            bson::doc! { "_id": *slug },
            bson::doc! {
                "$setOnInsert": {
                    "_id": *slug,
                    "question_text": *question_text,
                    "blurb": *blurb,
                    "sort_order": i as i32,
                    "status": "not_started",
                    "current_version_id": bson::Bson::Null,
                    "chat": bson::Bson::Array(Vec::new()),
                    "created_at": &now_str,
                    "updated_at": &now_str,
                },
            },
        )
        .upsert(true)
        .await?;

        // Mirror into the questions bank as a role-only Question with a
        // deterministic id so re-seeds are idempotent and a delete from the
        // /companies/.../questions UI is recoverable on next boot.
        let qid = pitch_question_id(slug);
        questions
            .update_one(
                bson::doc! { "_id": &qid },
                bson::doc! {
                    "$setOnInsert": {
                        "_id": &qid,
                        "text": *question_text,
                        "source": "pitch",
                        "role": Role::SolutionsArchitect.as_str(),
                        "categories": bson::Bson::Array(Vec::new()),
                        "company_id": bson::Bson::Null,
                        "added_at": &now_str,
                    },
                },
            )
            .upsert(true)
            .await?;
    }
    tracing::info!(count = SEED.len(), "seeded canonical pitches");
    Ok(())
}

/// Deterministic Question `_id` for a pitch — `"pitch-{slug}"`. Keeps the
/// seed idempotent and the question recoverable across boots.
pub fn pitch_question_id(slug: &str) -> String {
    format!("pitch-{slug}")
}

pub async fn list_all(db: &Database) -> Result<Vec<Pitch>, AppError> {
    let opts = FindOptions::builder()
        .sort(bson::doc! { "sort_order": 1 })
        .build();
    let cursor = db
        .collection::<Pitch>(Pitch::COLLECTION)
        .find(bson::doc! {})
        .with_options(opts)
        .await?;
    Ok(cursor.try_collect().await?)
}

pub async fn get(db: &Database, slug: &str) -> Result<Option<Pitch>, AppError> {
    Ok(db
        .collection::<Pitch>(Pitch::COLLECTION)
        .find_one(bson::doc! { "_id": slug })
        .await?)
}

/// Append a chat turn and bump `updated_at`. Mirrors the helper in
/// `routes::stories::mod::push_turn`.
pub async fn push_turn(db: &Database, pitch: &mut Pitch, turn: ChatTurn) -> Result<(), AppError> {
    let now = chrono::Utc::now();
    let turn_doc = bson::to_bson(&turn)?;
    db.collection::<Pitch>(Pitch::COLLECTION)
        .update_one(
            bson::doc! { "_id": &pitch.id },
            bson::doc! {
                "$push": { "chat": turn_doc },
                "$set":  {
                    "updated_at": now.to_rfc3339(),
                    "status": status_str(PitchStatus::InProgress),
                },
            },
        )
        .await?;
    pitch.chat.push(turn);
    pitch.updated_at = now;
    if matches!(pitch.status, PitchStatus::NotStarted) {
        pitch.status = PitchStatus::InProgress;
    }
    Ok(())
}

/// Update `current_version_id` and flip status to Locked. Called by
/// `pitch_lockin::generate_and_save` after a new version is inserted.
pub async fn set_current_version(
    db: &Database,
    pitch_id: &str,
    version_id: &str,
) -> Result<(), AppError> {
    db.collection::<Pitch>(Pitch::COLLECTION)
        .update_one(
            bson::doc! { "_id": pitch_id },
            bson::doc! { "$set": {
                "current_version_id": version_id,
                "status": status_str(PitchStatus::Locked),
                "updated_at": chrono::Utc::now().to_rfc3339(),
            } },
        )
        .await?;
    Ok(())
}

/// Reopen a locked pitch for refinement — flips status back to InProgress
/// so the next lock-in creates vN+1.
pub async fn reopen(db: &Database, pitch_id: &str) -> Result<(), AppError> {
    db.collection::<Pitch>(Pitch::COLLECTION)
        .update_one(
            bson::doc! { "_id": pitch_id },
            bson::doc! { "$set": {
                "status": status_str(PitchStatus::InProgress),
                "updated_at": chrono::Utc::now().to_rfc3339(),
            } },
        )
        .await?;
    Ok(())
}

/// Wipe chat and version pointer, return status to not_started. Used by
/// the delete-and-restart action on the show page.
pub async fn reset(db: &Database, pitch_id: &str) -> Result<(), AppError> {
    db.collection::<Pitch>(Pitch::COLLECTION)
        .update_one(
            bson::doc! { "_id": pitch_id },
            bson::doc! { "$set": {
                "status": status_str(PitchStatus::NotStarted),
                "current_version_id": bson::Bson::Null,
                "chat": bson::Bson::Array(Vec::new()),
                "updated_at": chrono::Utc::now().to_rfc3339(),
            } },
        )
        .await?;
    Ok(())
}

fn status_str(s: PitchStatus) -> &'static str {
    match s {
        PitchStatus::NotStarted => "not_started",
        PitchStatus::InProgress => "in_progress",
        PitchStatus::Locked => "locked",
    }
}

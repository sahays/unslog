//! Canonical pitch catalog — seeded on first run, state preserved across
//! restarts.
//!
//! Each seed row inserts only when the row is missing (`$setOnInsert`), so a
//! deployed app gets new pitch types when the seed list grows without
//! clobbering any chat the user has already accumulated on existing rows.

use futures::TryStreamExt;
use mongodb::options::FindOptions;
use mongodb::Database;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{ChatTurn, Pitch, PitchStatus, Role};
use crate::services::current_owner::TEMP_OWNER_ID;

/// Hard ceiling on the embedded chat array. Mirrors `story_store` — the
/// UX assumes a ~50-turn working set, the cap blocks pathological loops
/// well before the 16 MB Mongo doc limit. Hitting the cap surfaces a
/// 400 telling the user to lock in and continue from there.
const MAX_CHAT_TURNS: usize = 100;

/// Pure capacity guard for [`push_turn`]. Split out for unit testing
/// without a live `&Database` handle.
fn check_chat_capacity(current_len: usize) -> Result<(), AppError> {
    if current_len >= MAX_CHAT_TURNS {
        return Err(AppError::BadRequest(format!(
            "chat is full ({MAX_CHAT_TURNS} turns) — lock this version in and refine from there"
        )));
    }
    Ok(())
}

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

/// On startup, idempotently upsert each canonical pitch row (Mongo) and a
/// matching role-only Question for the bank (Postgres after Phase A.6).
/// Existing user state — Mongo pitch row's status / chat /
/// current_version_id, and any tags the user added to the mirrored
/// question — is preserved via `$setOnInsert` / `ON CONFLICT DO NOTHING`.
pub async fn seed_defaults(db: &Database, pool: &PgPool) -> Result<(), AppError> {
    let coll = db.collection::<Pitch>(Pitch::COLLECTION);
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
        seed_pitch_question_row(pool, slug, question_text, now).await?;
    }
    tracing::info!(count = SEED.len(), "seeded canonical pitches");
    Ok(())
}

/// Mirror one pitch slug into the Postgres `questions` table as a
/// role-only question. Idempotent via the PK conflict.
async fn seed_pitch_question_row(
    pool: &PgPool,
    slug: &str,
    text: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), AppError> {
    sqlx::query(
        r#"INSERT INTO questions
           (id, owner_id, text, source, role, categories, company_id, added_at)
           VALUES ($1, $2, $3, 'pitch', $4, '[]'::jsonb, NULL, $5)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(pitch_question_id(slug))
    .bind(TEMP_OWNER_ID)
    .bind(text)
    .bind(Role::SolutionsArchitect.as_str())
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Deterministic Question id for a pitch — `qstpitchN` where N is the
/// 1-based slot in [`SEED`]. Satisfies the Postgres CHECK
/// `^qst[a-z0-9]{6}$` (3-char prefix + 6 alphanum). Append-only growth
/// of SEED keeps ids stable across boots.
pub fn pitch_question_id(slug: &str) -> String {
    let slot = SEED
        .iter()
        .position(|(s, _, _)| *s == slug)
        .map_or(0, |i| i + 1);
    format!("qstpitch{slot}")
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

/// Tight projection for the home page's "In progress" feed — never need
/// the embedded chat or version pointer to render the row.
#[derive(serde::Deserialize)]
pub struct InProgressRow {
    #[serde(rename = "_id")]
    pub slug: String,
    pub question_text: String,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// In-progress pitches only, newest-touched-first.
pub async fn list_in_progress(db: &Database) -> Result<Vec<InProgressRow>, AppError> {
    let opts = FindOptions::builder()
        .sort(bson::doc! { "updated_at": -1 })
        .projection(bson::doc! { "_id": 1, "question_text": 1, "updated_at": 1 })
        .build();
    let cursor = db
        .collection::<InProgressRow>(Pitch::COLLECTION)
        .find(bson::doc! { "status": PitchStatus::InProgress.as_str() })
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
/// `services::story_store::push_turn`. Refuses to grow the embedded
/// `chat[]` past [`MAX_CHAT_TURNS`] turns — the caller should lock in
/// a version and start a refine chat from a clean slate.
pub async fn push_turn(db: &Database, pitch: &mut Pitch, turn: ChatTurn) -> Result<(), AppError> {
    check_chat_capacity(pitch.chat.len())?;
    let now = chrono::Utc::now();
    let turn_doc = bson::to_bson(&turn)?;
    db.collection::<Pitch>(Pitch::COLLECTION)
        .update_one(
            bson::doc! { "_id": &pitch.id },
            bson::doc! {
                "$push": { "chat": turn_doc },
                "$set":  {
                    "updated_at": now.to_rfc3339(),
                    "status": PitchStatus::InProgress.as_str(),
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
                "status": PitchStatus::Locked.as_str(),
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
                "status": PitchStatus::InProgress.as_str(),
                "updated_at": chrono::Utc::now().to_rfc3339(),
            } },
        )
        .await?;
    Ok(())
}

/// Sibling-pitch projection used by the show-page sidebar quick-jump panel.
/// Slim row — only what the template renders.
pub struct SiblingRow {
    pub slug: String,
    pub question_text: String,
    pub status: PitchStatus,
}

/// All other pitches (everything except `pitch_id`), sorted by `sort_order`.
/// Drives the sidebar on `/pitches/:slug`.
pub async fn list_siblings(db: &Database, pitch_id: &str) -> Result<Vec<SiblingRow>, AppError> {
    #[derive(serde::Deserialize)]
    struct Row {
        #[serde(rename = "_id")]
        id: String,
        question_text: String,
        status: PitchStatus,
    }
    let opts = FindOptions::builder()
        .sort(bson::doc! { "sort_order": 1 })
        .projection(bson::doc! { "_id": 1, "question_text": 1, "status": 1 })
        .build();
    let cursor = db
        .collection::<Row>(Pitch::COLLECTION)
        .find(bson::doc! { "_id": { "$ne": pitch_id } })
        .with_options(opts)
        .await?;
    let rows: Vec<Row> = cursor.try_collect().await?;
    Ok(rows
        .into_iter()
        .map(|r| SiblingRow {
            slug: r.id,
            question_text: r.question_text,
            status: r.status,
        })
        .collect())
}

/// Wipe chat and version pointer, return status to not_started. Used by
/// the delete-and-restart action on the show page.
pub async fn reset(db: &Database, pitch_id: &str) -> Result<(), AppError> {
    db.collection::<Pitch>(Pitch::COLLECTION)
        .update_one(
            bson::doc! { "_id": pitch_id },
            bson::doc! { "$set": {
                "status": PitchStatus::NotStarted.as_str(),
                "current_version_id": bson::Bson::Null,
                "chat": bson::Bson::Array(Vec::new()),
                "updated_at": chrono::Utc::now().to_rfc3339(),
            } },
        )
        .await?;
    Ok(())
}

#[cfg(test)]
#[path = "pitch_store_tests.rs"]
mod tests;

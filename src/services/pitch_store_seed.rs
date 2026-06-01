//! Catalog seed for [`crate::services::pitch_store`] — pulled into its own
//! file so the main store stays under the per-file LOC budget. The SEED
//! tuples are the source of truth for both the `pitches` catalog rows and
//! the mirrored role-only `questions` rows.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::Role;
use crate::services::master_seed::MASTER_ID;

/// Seven canonical intro/narrative questions. Slug = catalog id. Tuple is
/// `(slug, question_text, blurb)`. Index drives `sort_order`, so the
/// order here is the order shown on the tile grid.
pub(super) const SEED: &[(&str, &str, &str)] = &[
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

/// On startup, idempotently insert each canonical pitch row in the
/// Postgres catalog and a matching role-only Question for the bank. The
/// catalog never overwrites; per-user state on `pitch_user_state` is
/// untouched.
pub async fn seed_defaults(pool: &PgPool) -> Result<(), AppError> {
    let now = chrono::Utc::now();
    for (i, (slug, question_text, blurb)) in SEED.iter().enumerate() {
        sqlx::query(
            r#"INSERT INTO pitches (id, question_text, blurb, sort_order, created_at)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(*slug)
        .bind(*question_text)
        .bind(*blurb)
        .bind(i as i32)
        .bind(now)
        .execute(pool)
        .await?;
        seed_pitch_question_row(pool, slug, question_text, now).await?;
    }
    tracing::info!(count = SEED.len(), "seeded canonical pitches");
    Ok(())
}

/// Mirror one pitch slug into the Postgres `questions` table as a
/// role-only question. Idempotent via the PK conflict.
///
/// The seeded Question is intentionally pinned to the master user. Per
/// migration 0003 only `questions`/`companies`/`sessions` rows carry
/// `owner_id`, and these seeded pitch-mirrored Questions are intended as
/// a global catalog row visible to every account that practices via
/// `/practice` — owner_id is therefore set to MASTER_ID by design.
async fn seed_pitch_question_row(
    pool: &PgPool,
    slug: &str,
    text: &str,
    now: DateTime<Utc>,
) -> Result<(), AppError> {
    sqlx::query(
        r#"INSERT INTO questions
           (id, owner_id, text, source, role, categories, company_id, added_at)
           VALUES ($1, $2, $3, 'pitch', $4, '[]'::jsonb, NULL, $5)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(pitch_question_id(slug))
    .bind(MASTER_ID)
    .bind(text)
    .bind(Role::SolutionsArchitect.as_str())
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Deterministic Question id for a pitch — `qstpitchN` where N is the
/// 1-based slot in [`SEED`]. Satisfies the Postgres CHECK
/// `^qst[a-z0-9]{6}$` (3-char prefix + 6 alphanum). Append-only growth of
/// SEED keeps ids stable across boots.
pub fn pitch_question_id(slug: &str) -> String {
    let slot = SEED
        .iter()
        .position(|(s, _, _)| *s == slug)
        .map_or(0, |i| i + 1);
    format!("qstpitch{slot}")
}

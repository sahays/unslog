//! Session curator — picks 4–6 questions in order at session start using the
//! cheap `lite_model`. Inputs include the candidate question pool, the user's
//! recent recurring weaknesses (extracted from the last few session
//! summaries for this role), and recently-asked question IDs to skip.
//!
//! Output is an ordered list of question IDs + a short "today's focus" line
//! shown on the active session header. On any LLM failure we fall back to a
//! random shuffle so the user can still start practising.

use futures::TryStreamExt;
use mongodb::options::FindOptions;
use mongodb::Database;
use rand::seq::SliceRandom;
use serde::Deserialize;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{Category, Question, Role, Session, SessionStatus, Summary};
use crate::services::category_store;
use crate::services::openrouter::{parse_json, ChatMessage, LlmClient};
use crate::services::questions;

const TARGET_MIN: usize = 4;
const TARGET_MAX: usize = 6;
const TARGET_DEFAULT: usize = 5;
/// How far back to look for "recent" sessions when computing weakness bias
/// and the recently-asked exclusion list.
const RECENT_SESSIONS: i64 = 3;

#[derive(Debug, Clone)]
pub struct CuratorOutput {
    pub question_ids: Vec<String>,
    pub focus_line: String,
}

#[derive(Debug, Deserialize)]
struct CuratorJson {
    #[serde(default)]
    question_ids: Vec<String>,
    #[serde(default)]
    focus_line: String,
}

/// Pick the curated session for a (role, selected_companies) tuple.
pub async fn curate(
    or: &dyn LlmClient,
    db: &Database,
    pg: &PgPool,
    lite_model: &str,
    role: Role,
    selected_company_ids: &[String],
) -> Result<CuratorOutput, AppError> {
    let span = tracing::info_span!(
        "curator",
        model = lite_model,
        role = role.as_str(),
        company_n = selected_company_ids.len(),
    );
    let _enter = span.enter();
    let start = std::time::Instant::now();

    let pool = load_pool(pg, role, selected_company_ids).await?;
    if pool.is_empty() {
        return Err(AppError::BadRequest(format!(
            "no {} questions in the selected companies — add some first",
            role.display_name()
        )));
    }

    // TODO(phase-A.7): re-target to Postgres summaries/evaluations once
    // those tables are ported. Until then `db` still serves recent
    // summaries and the recently-asked-questions exclusion set.
    let recent_summaries = recent_summaries(db, selected_company_ids).await?;
    let recently_asked = recently_asked_ids(db, selected_company_ids, role).await?;
    let canonical = category_store::list_all(pg).await?;

    let llm_attempt = try_llm_curate(
        or,
        lite_model,
        &pool,
        &recent_summaries,
        &recently_asked,
        &canonical,
    )
    .await;

    let (picked_ids, focus_line) = match llm_attempt {
        Ok(out) if !out.question_ids.is_empty() => {
            // Validate: every returned ID must actually be in our pool, and
            // we cap at TARGET_MAX. Drop unknown IDs.
            let pool_ids: std::collections::HashSet<&str> =
                pool.iter().map(|p| p.id.as_str()).collect();
            let validated: Vec<String> = out
                .question_ids
                .into_iter()
                .filter(|id| pool_ids.contains(id.as_str()))
                .take(TARGET_MAX)
                .collect();
            if validated.is_empty() {
                tracing::warn!("curator returned no valid IDs from pool, falling back to random");
                fallback_random(&pool, &recently_asked)
            } else {
                (validated, out.focus_line)
            }
        }
        Ok(_) => {
            tracing::warn!("curator returned empty picks, falling back to random");
            fallback_random(&pool, &recently_asked)
        }
        Err(e) => {
            tracing::warn!(error = %e, "curator LLM call failed, falling back to random");
            fallback_random(&pool, &recently_asked)
        }
    };

    tracing::info!(
        op = "curator",
        duration_ms = start.elapsed().as_millis() as u64,
        pool_size = pool.len(),
        picks = picked_ids.len(),
        recently_asked_n = recently_asked.len(),
        weaknesses_n = recent_summaries
            .iter()
            .map(|s| s.recurring_weaknesses.len())
            .sum::<usize>(),
        "curator done",
    );

    Ok(CuratorOutput {
        question_ids: picked_ids,
        focus_line,
    })
}

/// Lightweight projection of a candidate question — just what the curator
/// (and LLM prompt) need.
#[derive(Debug, Clone)]
pub struct PoolItem {
    pub id: String,
    pub text: String,
    pub company_id: String,
    pub categories: Vec<String>,
}

async fn load_pool(
    pg: &PgPool,
    role: Role,
    selected_company_ids: &[String],
) -> Result<Vec<PoolItem>, AppError> {
    let pool: Vec<Question> = questions::list_for_pool(pg, role, selected_company_ids).await?;
    Ok(pool
        .into_iter()
        .map(|q| PoolItem {
            id: q.id,
            text: q.text,
            company_id: q.company_id.unwrap_or_default(),
            categories: q.categories,
        })
        .collect())
}

async fn recent_summaries(
    db: &Database,
    selected_company_ids: &[String],
) -> Result<Vec<Summary>, AppError> {
    if selected_company_ids.is_empty() {
        return Ok(Vec::new());
    }
    let opts = FindOptions::builder()
        .sort(bson::doc! { "created_at": -1 })
        .limit(RECENT_SESSIONS)
        .build();
    let cursor = db
        .collection::<Summary>(Summary::COLLECTION)
        .find(bson::doc! { "company_id": { "$in": selected_company_ids } })
        .with_options(opts)
        .await?;
    Ok(cursor.try_collect().await?)
}

async fn recently_asked_ids(
    db: &Database,
    selected_company_ids: &[String],
    role: Role,
) -> Result<std::collections::HashSet<String>, AppError> {
    use crate::models::Evaluation;
    if selected_company_ids.is_empty() {
        return Ok(std::collections::HashSet::new());
    }
    // Pull the last N sessions for these companies + role, then collect
    // every question_id their evaluations referenced. Both reads project
    // the minimal column set — Evaluation embeds an `attempts[]` array
    // that can run multi-KB per row.
    let opts = FindOptions::builder()
        .sort(bson::doc! { "started_at": -1 })
        .limit(RECENT_SESSIONS * 2)
        .projection(bson::doc! { "_id": 1 })
        .build();
    #[derive(serde::Deserialize)]
    struct SidRow {
        #[serde(rename = "_id")]
        id: String,
    }
    let session_ids: Vec<String> = db
        .collection::<SidRow>(Session::COLLECTION)
        .find(bson::doc! {
            "company_id": { "$in": selected_company_ids },
            "role": role.as_str(),
            "status": SessionStatus::Ended.as_str(),
        })
        .with_options(opts)
        .await?
        .try_collect::<Vec<SidRow>>()
        .await?
        .into_iter()
        .map(|r| r.id)
        .collect();
    if session_ids.is_empty() {
        return Ok(std::collections::HashSet::new());
    }
    #[derive(serde::Deserialize)]
    struct QidRow {
        question_id: String,
    }
    let eval_opts = FindOptions::builder()
        .projection(bson::doc! { "question_id": 1 })
        .build();
    let qids: Vec<QidRow> = db
        .collection::<QidRow>(Evaluation::COLLECTION)
        .find(bson::doc! { "session_id": { "$in": &session_ids } })
        .with_options(eval_opts)
        .await?
        .try_collect()
        .await?;
    Ok(qids.into_iter().map(|r| r.question_id).collect())
}

async fn try_llm_curate(
    or: &dyn LlmClient,
    lite_model: &str,
    pool: &[PoolItem],
    recent_summaries: &[Summary],
    recently_asked: &std::collections::HashSet<String>,
    canonical: &[Category],
) -> Result<CuratorJson, AppError> {
    let canonical_block = canonical
        .iter()
        .map(|c| format!("- `{}`: {}", c.id, c.name))
        .collect::<Vec<_>>()
        .join("\n");

    let pool_block = pool
        .iter()
        .map(|p| {
            let cats = if p.categories.is_empty() {
                "(untagged)".to_string()
            } else {
                p.categories.join(", ")
            };
            let recent_marker = if recently_asked.contains(&p.id) {
                " (RECENTLY ASKED — avoid unless pool is too small)"
            } else {
                ""
            };
            format!(
                "- {} | categories: [{cats}] | text: {}{recent_marker}",
                p.id, p.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let weaknesses_block = if recent_summaries.is_empty() {
        "(no prior session summaries — no weakness bias)".to_string()
    } else {
        recent_summaries
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let weak = if s.recurring_weaknesses.is_empty() {
                    "(none recorded)".to_string()
                } else {
                    s.recurring_weaknesses.join("; ")
                };
                format!("Session -{} weaknesses: {}", i + 1, weak)
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let target_n = TARGET_DEFAULT;

    let system = "You curate a focused behavioral-interview practice session. From a candidate question pool, pick a tight ordered set the user will actually benefit from: bias toward categories where past sessions showed recurring weaknesses, avoid recently-asked questions if alternatives exist, diversify categories so the same competency isn't tested twice in a row, and order the picks to build narrative coherence (warm up, escalate, reflective close).";

    let user = format!(
        r#"<canonical_categories>
{canonical_block}
</canonical_categories>

<pool>
{pool_block}
</pool>

<recent_weaknesses>
{weaknesses_block}
</recent_weaknesses>

Pick {TARGET_MIN}–{TARGET_MAX} questions (target {target_n}) from the pool, in answer order. Return ONLY a JSON object with this shape:

{{
  "question_ids": ["<id-from-pool>", "<id-from-pool>", ...],
  "focus_line": "<one short sentence describing today's focus, e.g. 'Today's focus is ambiguity, where you stumbled last session, plus a fresh ownership story for breadth.'>"
}}

Rules:
- Use ONLY question IDs that appear in the pool above. No fabricated IDs.
- {TARGET_MIN} ≤ count ≤ {TARGET_MAX}.
- Never repeat an ID.
- If the pool is small (< {TARGET_MIN}), include every available question."#
    );

    let raw = or
        .chat(
            lite_model,
            vec![ChatMessage::system(system), ChatMessage::user(user)],
            true,
        )
        .await?;

    parse_json(&raw)
}

fn fallback_random(
    pool: &[PoolItem],
    recently_asked: &std::collections::HashSet<String>,
) -> (Vec<String>, String) {
    let mut rng = rand::thread_rng();
    let mut fresh: Vec<&PoolItem> = pool
        .iter()
        .filter(|p| !recently_asked.contains(&p.id))
        .collect();
    if fresh.len() < TARGET_MIN {
        // Pool is exhausted of fresh questions — allow recently-asked.
        fresh = pool.iter().collect();
    }
    fresh.shuffle(&mut rng);
    let take = fresh.len().clamp(TARGET_MIN.min(fresh.len()), TARGET_MAX);
    let picks: Vec<String> = fresh.into_iter().take(take).map(|p| p.id.clone()).collect();
    (picks, "Random selection — curator unavailable.".to_string())
}

/// Helper used by `next_question`: walk the curated list and return the next
/// unanswered ID. Returns None when the list is exhausted.
pub fn next_curated(
    session: &Session,
    answered_ids: &std::collections::HashSet<String>,
) -> Option<String> {
    if matches!(session.status, SessionStatus::Ended) {
        return None;
    }
    session
        .curated_question_ids
        .iter()
        .find(|id| !answered_ids.contains(id.as_str()))
        .cloned()
}

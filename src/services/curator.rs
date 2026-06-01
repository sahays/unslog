//! Session curator — picks 4–6 questions in order at session start using the
//! cheap `lite_model`. Inputs include the candidate question pool, the user's
//! recent recurring weaknesses (extracted from the last few session
//! summaries for this role), and recently-asked question IDs to skip.
//!
//! Output is an ordered list of question IDs + a short "today's focus" line
//! shown on the active session header. On any LLM failure we fall back to a
//! random shuffle so the user can still start practising.
//!
//! Postgres reads live in [`mod db`]; LLM prompt assembly in [`mod prompt`].

mod db;
mod prompt;

use std::collections::HashSet;

use rand::seq::SliceRandom;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{Question, Role, Session, SessionStatus};
use crate::services::category_store;
use crate::services::openrouter::LlmClient;
use crate::services::questions;

use prompt::{CuratorJson, TARGET_MAX, TARGET_MIN};

#[derive(Debug, Clone)]
pub struct CuratorOutput {
    pub question_ids: Vec<String>,
    pub focus_line: String,
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

/// Pick the curated session for a (role, selected_companies) tuple.
pub async fn curate(
    or: &dyn LlmClient,
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

    let recent_summaries = db::recent_summaries(pg, selected_company_ids).await?;
    let recently_asked = db::recently_asked_ids(pg, selected_company_ids, role).await?;
    let canonical = category_store::list_all(pg).await?;

    let llm_attempt = prompt::try_llm_curate(
        or,
        lite_model,
        &pool,
        &recent_summaries,
        &recently_asked,
        &canonical,
    )
    .await;

    let (picked_ids, focus_line) = choose_picks(llm_attempt, &pool, &recently_asked);

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

fn choose_picks(
    llm_attempt: Result<CuratorJson, AppError>,
    pool: &[PoolItem],
    recently_asked: &HashSet<String>,
) -> (Vec<String>, String) {
    match llm_attempt {
        Ok(out) if !out.question_ids.is_empty() => validate_or_fallback(out, pool, recently_asked),
        Ok(_) => {
            tracing::warn!("curator returned empty picks, falling back to random");
            fallback_random(pool, recently_asked)
        }
        Err(e) => {
            tracing::warn!(error = %e, "curator LLM call failed, falling back to random");
            fallback_random(pool, recently_asked)
        }
    }
}

fn validate_or_fallback(
    out: CuratorJson,
    pool: &[PoolItem],
    recently_asked: &HashSet<String>,
) -> (Vec<String>, String) {
    let pool_ids: HashSet<&str> = pool.iter().map(|p| p.id.as_str()).collect();
    let validated: Vec<String> = out
        .question_ids
        .into_iter()
        .filter(|id| pool_ids.contains(id.as_str()))
        .take(TARGET_MAX)
        .collect();
    if validated.is_empty() {
        tracing::warn!("curator returned no valid IDs from pool, falling back to random");
        fallback_random(pool, recently_asked)
    } else {
        (validated, out.focus_line)
    }
}

fn fallback_random(pool: &[PoolItem], recently_asked: &HashSet<String>) -> (Vec<String>, String) {
    let mut rng = rand::thread_rng();
    let mut fresh: Vec<&PoolItem> = pool
        .iter()
        .filter(|p| !recently_asked.contains(&p.id))
        .collect();
    if fresh.len() < TARGET_MIN {
        fresh = pool.iter().collect();
    }
    fresh.shuffle(&mut rng);
    let take = fresh.len().clamp(TARGET_MIN.min(fresh.len()), TARGET_MAX);
    let picks: Vec<String> = fresh.into_iter().take(take).map(|p| p.id.clone()).collect();
    (picks, "Random selection — curator unavailable.".to_string())
}

/// Helper used by `next_question`: walk the curated list and return the next
/// unanswered ID. Returns None when the list is exhausted.
pub fn next_curated(session: &Session, answered_ids: &HashSet<String>) -> Option<String> {
    if matches!(session.status, SessionStatus::Ended) {
        return None;
    }
    session
        .curated_question_ids
        .iter()
        .find(|id| !answered_ids.contains(id.as_str()))
        .cloned()
}

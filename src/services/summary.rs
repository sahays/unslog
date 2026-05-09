//! End-of-session debrief.
//!
//! Loads the snapshotted summary prompt + this session's evaluations + last few
//! prior summaries for the same company, calls OpenRouter, parses JSON, and
//! persists a Summary row keyed by session_id (one summary per session).

use futures::TryStreamExt;
use mongodb::options::FindOptions;
use mongodb::Database;

use crate::error::AppError;
use crate::models::{Company, Evaluation, Session, Summary, SummaryPayload};
use crate::services::{
    openrouter::{ChatMessage, OpenRouter},
    prompt_store,
};

/// How many prior session summaries to carry into the new summary's prompt.
const MAX_PRIOR_SUMMARIES: i64 = 3;

/// How many prior summary narratives to surface for *future* critique calls.
pub const CARRY_FORWARD_INTO_CRITIQUE: i64 = 3;

/// Generate, persist, and return the summary for `session`. Idempotent: if a
/// summary already exists for this session, returns it unchanged.
pub async fn generate_and_save(
    or: &OpenRouter,
    db: &Database,
    session: &Session,
    company: &Company,
) -> Result<Summary, AppError> {
    let span = tracing::info_span!(
        "summary",
        session_id = %session.id,
        company_id = %company.id,
        model = %session.model_snapshot.critique,
    );
    let _enter = span.enter();
    let start = std::time::Instant::now();

    let summaries = db.collection::<Summary>(Summary::COLLECTION);
    if let Some(existing) = summaries
        .find_one(bson::doc! { "session_id": &session.id })
        .await?
    {
        tracing::info!(op = "summary", reused = true, "existing summary returned");
        return Ok(existing);
    }

    // Snapshotted summary prompt.
    let summary_prompt = prompt_store::get_version(db, &session.prompt_snapshot.summary)
        .await?
        .ok_or_else(|| AppError::NotFound("summary prompt version".into()))?;

    // This session's evaluations, ordered as they were taken.
    let evals: Vec<Evaluation> = db
        .collection::<Evaluation>(Evaluation::COLLECTION)
        .find(bson::doc! { "session_id": &session.id })
        .with_options(FindOptions::builder().sort(bson::doc! { "_id": 1 }).build())
        .await?
        .try_collect()
        .await?;

    // Prior summaries for this company (excluding this session, in case we
    // re-run end on a session that was rolled back).
    let priors =
        recent_company_summaries(db, &company.id, Some(&session.id), MAX_PRIOR_SUMMARIES).await?;

    let user = render_user_message(company, &evals, &priors);

    let raw = or
        .chat(
            &session.model_snapshot.critique,
            vec![
                ChatMessage::system(summary_prompt.body),
                ChatMessage::user(user),
            ],
            true,
        )
        .await?;

    let payload: SummaryPayload = crate::services::openrouter::parse_json(&raw).map_err(|e| {
        tracing::warn!(error = %e, raw_preview = %preview(&raw, 240), "summary JSON parse failed");
        AppError::Upstream(format!(
            "summary returned invalid JSON: {e} — raw: {}",
            preview(&raw, 280)
        ))
    })?;

    let summary = Summary {
        id: uuid::Uuid::now_v7().to_string(),
        session_id: session.id.clone(),
        company_id: company.id.clone(),
        narrative: payload.narrative,
        strengths: payload.strengths,
        recurring_weaknesses: payload.recurring_weaknesses,
        blind_spots: payload.blind_spots,
        company_fit_signal: payload.company_fit_signal,
        created_at: chrono::Utc::now(),
    };
    summaries.insert_one(&summary).await?;
    tracing::info!(
        op = "summary",
        duration_ms = start.elapsed().as_millis() as u64,
        evals_n = evals.len(),
        priors_n = priors.len(),
        "summary saved",
    );

    Ok(summary)
}

/// Last `limit` summary narratives for `company_id`, newest first. Used both
/// inside the summary prompt itself (priors) and inside future critique calls
/// (carry-forward context).
pub async fn recent_company_summaries(
    db: &Database,
    company_id: &str,
    exclude_session: Option<&str>,
    limit: i64,
) -> Result<Vec<Summary>, AppError> {
    let mut filter = bson::doc! { "company_id": company_id };
    if let Some(sid) = exclude_session {
        filter.insert("session_id", bson::doc! { "$ne": sid });
    }
    let opts = FindOptions::builder()
        .sort(bson::doc! { "created_at": -1 })
        .limit(limit)
        .build();
    let cursor = db
        .collection::<Summary>(Summary::COLLECTION)
        .find(filter)
        .with_options(opts)
        .await?;
    Ok(cursor.try_collect().await?)
}

/// Lookup-by-session helper for the per-session view.
pub async fn for_session(db: &Database, session_id: &str) -> Result<Option<Summary>, AppError> {
    Ok(db
        .collection::<Summary>(Summary::COLLECTION)
        .find_one(bson::doc! { "session_id": session_id })
        .await?)
}

fn render_user_message(company: &Company, evals: &[Evaluation], priors: &[Summary]) -> String {
    let packet = company_packet(company);

    let priors_block = if priors.is_empty() {
        "(no prior session summaries for this company yet)".to_string()
    } else {
        priors
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let weak = if s.recurring_weaknesses.is_empty() {
                    String::new()
                } else {
                    format!(
                        "\nrecurring_weaknesses: {}",
                        s.recurring_weaknesses.join("; ")
                    )
                };
                let blind = if s.blind_spots.is_empty() {
                    String::new()
                } else {
                    format!("\nblind_spots: {}", s.blind_spots.join("; "))
                };
                format!(
                    "Session -{} ({}):\n{}{}{}",
                    i + 1,
                    s.created_at.format("%Y-%m-%d"),
                    s.narrative,
                    weak,
                    blind,
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n---\n\n")
    };

    let evals_block = if evals.is_empty() {
        "(no questions answered in this session)".to_string()
    } else {
        evals
            .iter()
            .enumerate()
            .map(|(i, e)| render_eval(i + 1, e))
            .collect::<Vec<_>>()
            .join("\n\n===\n\n")
    };

    format!(
        r#"<company_packet>
{packet}
</company_packet>

<prior_summaries>
{priors_block}
</prior_summaries>

<this_session>
{evals_block}
</this_session>

Write the debrief now. Return only the JSON object — no other text, no fences."#
    )
}

fn render_eval(idx: usize, e: &Evaluation) -> String {
    let attempts = if e.attempts.is_empty() {
        "(no attempts — question was skipped)".to_string()
    } else {
        e.attempts
            .iter()
            .map(|a| {
                let crit = match &a.critique {
                    Some(c) => {
                        let fit = c
                            .scores
                            .company_fit
                            .map(|v| format!("{v}/5"))
                            .unwrap_or_else(|| "n/a".to_string());
                        format!(
                            "\nScores: spec {}/5, role {}/5, STAR+ {}/5, pitfalls {}/5, fit {}\nCritique: {}",
                            c.scores.specificity,
                            c.scores.role_clarity,
                            c.scores.star_plus_structure,
                            c.scores.pitfalls_avoided,
                            fit,
                            c.narrative
                        )
                    }
                    None => String::new(),
                };
                format!(
                    "Attempt {}:\nAnswer: {}{}",
                    a.attempt_n, a.answer_transcript, crit
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    };

    format!("Q{idx}. {}\n\n{attempts}", e.question_text)
}

fn company_packet(c: &Company) -> String {
    let header = format!("Company: {}\nRole: {}", c.name, c.role);
    match &c.research_packet {
        Some(p) => format!(
            "{header}\n\nSummary:\n{}\n\nValues signal:\n{}",
            p.summary, p.values_signal
        ),
        None => header,
    }
}

fn preview(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "…"
    }
}

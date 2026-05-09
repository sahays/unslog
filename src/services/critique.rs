//! Critique pipeline: assemble prompt, call OpenRouter, parse JSON.

use mongodb::Database;

use crate::error::AppError;
use crate::models::{Attempt, Company, Critique, Session};
use crate::services::{
    assets::BookCache,
    openrouter::{self, ChatMessage, OpenRouter},
    prompt_store,
};

const MAX_PRIOR_SUMMARIES: usize = 3;

/// Build the critique prompt content (system + user messages) for a given attempt.
#[allow(clippy::too_many_arguments)]
pub async fn build_messages(
    db: &Database,
    book_cache: &BookCache,
    session: &Session,
    company: &Company,
    question_text: &str,
    new_answer: &str,
    prior_attempts: &[Attempt],
    prior_summary_narratives: &[String],
) -> Result<Vec<ChatMessage>, AppError> {
    // 1. System message — load by version_id from the session snapshot.
    let critique_prompt = prompt_store::get_version(db, &session.prompt_snapshot.critique)
        .await?
        .ok_or_else(|| AppError::NotFound("critique prompt version".into()))?;

    // 2. Book excerpts — primary asset's extracted text (cached).
    let book = book_cache.get(db).await?;

    // 3. Company packet
    let packet_str = render_packet(company);

    // 4. Prior summaries
    let prior_summaries_block = if prior_summary_narratives.is_empty() {
        "(no prior session summaries yet)".to_string()
    } else {
        prior_summary_narratives
            .iter()
            .take(MAX_PRIOR_SUMMARIES)
            .enumerate()
            .map(|(i, s)| format!("Session -{}:\n{}", i + 1, s))
            .collect::<Vec<_>>()
            .join("\n\n")
    };

    // 5. Prior attempts within this session for this question
    let attempts_block = if prior_attempts.is_empty() {
        "(this is attempt 1 — no prior attempts in this session)".to_string()
    } else {
        prior_attempts
            .iter()
            .map(|a| {
                let crit = a
                    .critique
                    .as_ref()
                    .map(|c| format!("Critique:\n{}", c.narrative))
                    .unwrap_or_default();
                format!(
                    "Attempt {} (transcript):\n{}\n\n{}",
                    a.attempt_n, a.answer_transcript, crit
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n---\n\n")
    };

    let next_attempt_n = (prior_attempts.len() as u32) + 1;

    let user = format!(
        r#"<book_excerpts>
{book}
</book_excerpts>

<company_packet>
{packet_str}
</company_packet>

<prior_summaries>
{prior_summaries_block}
</prior_summaries>

<question>
{question_text}
</question>

<attempts_in_this_session>
{attempts_block}
</attempts_in_this_session>

<new_attempt>
{new_answer}
</new_attempt>

This is attempt {next_attempt_n}. Produce the critique now. Return only the JSON object — no other text, no fences."#
    );

    Ok(vec![
        ChatMessage::system(critique_prompt.body),
        ChatMessage::user(user),
    ])
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    or: &OpenRouter,
    db: &Database,
    book_cache: &BookCache,
    session: &Session,
    company: &Company,
    question_text: &str,
    new_answer: &str,
    prior_attempts: &[Attempt],
    prior_summary_narratives: &[String],
) -> Result<Critique, AppError> {
    let attempt_n = (prior_attempts.len() as u32) + 1;
    let span = tracing::info_span!(
        "critique",
        session_id = %session.id,
        company_id = %company.id,
        model = %session.model_snapshot.critique,
        attempt_n,
    );
    let _enter = span.enter();
    let start = std::time::Instant::now();

    let messages = build_messages(
        db,
        book_cache,
        session,
        company,
        question_text,
        new_answer,
        prior_attempts,
        prior_summary_narratives,
    )
    .await?;

    let raw = or
        .chat(&session.model_snapshot.critique, messages, true)
        .await?;

    let critique: Critique = crate::services::openrouter::parse_json(&raw).map_err(|e| {
        tracing::warn!(error = %e, raw_preview = %openrouter::preview(&raw, 240), "critique JSON parse failed");
        AppError::Upstream(format!(
            "critique returned invalid JSON: {e} — raw: {}",
            openrouter::preview(&raw, 280)
        ))
    })?;

    tracing::info!(
        op = "critique",
        duration_ms = start.elapsed().as_millis() as u64,
        avg_score = critique.scores.average(),
        citations_n = critique.citations.len(),
        "critique done",
    );

    Ok(critique)
}

fn render_packet(company: &Company) -> String {
    let header = format!("Company: {}\nRole: {}\n", company.name, company.role);
    match &company.research_packet {
        Some(p) => format!(
            "{header}\nSummary:\n{}\n\nRole JD:\n{}\n\nValues signal:\n{}\n\nSample questions:\n{}\n\nSources:\n{}",
            p.summary,
            p.role_jd,
            p.values_signal,
            p.sample_questions
                .iter()
                .map(|q| format!("- {q}"))
                .collect::<Vec<_>>()
                .join("\n"),
            p.sources
                .iter()
                .map(|s| format!("- {} ({})", s.title, s.url))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        None => format!("{header}\n(no research packet — proceed with role-name signal only)"),
    }
}

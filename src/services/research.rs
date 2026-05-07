//! Research agent — produces a per-company packet.

use serde::Deserialize;

use crate::error::AppError;
use crate::models::company::{ResearchPacket, ResearchSource};
use crate::services::openrouter::{
    parse_json, ChatMessage, OpenRouter, DEFAULT_RESEARCH_MODEL,
};
use crate::services::prompt_store;

#[derive(Debug, Deserialize)]
struct AgentOutput {
    summary: String,
    role_jd: String,
    values_signal: String,
    #[serde(default)]
    sample_questions: Vec<String>,
    #[serde(default)]
    sources: Vec<ResearchSource>,
}

pub async fn run(
    or: &OpenRouter,
    db: &mongodb::Database,
    company_name: &str,
    role: &str,
) -> Result<ResearchPacket, AppError> {
    let prompt = prompt_store::get_prompt(db, "research")
        .await?
        .ok_or_else(|| AppError::NotFound("research prompt".into()))?;
    let body = prompt_store::get_current_body(db, "research").await?;

    let user = format!(
        "Company: {company_name}\nRole: {role}\n\nProduce the research packet now. Return only the JSON object specified, no other text."
    );

    let raw = or
        .chat(
            DEFAULT_RESEARCH_MODEL,
            vec![ChatMessage::system(body), ChatMessage::user(user)],
            true,
        )
        .await?;

    let out: AgentOutput = parse_json(&raw).map_err(|e| {
        AppError::Upstream(format!(
            "research agent returned invalid JSON: {e} — raw: {}",
            preview(&raw, 240)
        ))
    })?;

    Ok(ResearchPacket {
        summary: out.summary,
        role_jd: out.role_jd,
        values_signal: out.values_signal,
        sample_questions: out.sample_questions,
        sources: out.sources,
        research_prompt_version_id: prompt.current_version_id,
        last_refreshed_at: chrono::Utc::now(),
    })
}

fn preview(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "…"
    }
}

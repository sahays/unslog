//! Research agent — produces a per-company packet.

use serde::Deserialize;

use crate::error::AppError;
use crate::models::company::{ResearchPacket, ResearchSource};
use crate::services::openrouter::{self, parse_json, ChatMessage, LlmClient};
use crate::services::{prompt_store, settings_store};

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
    or: &dyn LlmClient,
    db: &mongodb::Database,
    company_name: &str,
    role: &str,
) -> Result<ResearchPacket, AppError> {
    let settings = settings_store::load(db).await?;
    let span = tracing::info_span!(
        "research",
        company = %company_name,
        role,
        model = %settings.research_model,
    );
    let _enter = span.enter();
    let start = std::time::Instant::now();

    let prompt = prompt_store::get_prompt(db, "research")
        .await?
        .ok_or_else(|| AppError::NotFound("research prompt".into()))?;
    let body = prompt_store::get_current_body(db, "research").await?;

    let user = format!(
        "Company: {company_name}\nRole: {role}\n\nProduce the research packet now. Return only the JSON object specified, no other text."
    );

    let raw = or
        .chat(
            &settings.research_model,
            vec![ChatMessage::system(body), ChatMessage::user(user)],
            true,
        )
        .await?;

    let out: AgentOutput = parse_json(&raw).map_err(|e| {
        tracing::warn!(error = %e, raw_preview = %openrouter::preview(&raw, 240), "research JSON parse failed");
        AppError::Upstream(format!(
            "research agent returned invalid JSON: {e} — raw: {}",
            openrouter::preview(&raw, 240)
        ))
    })?;

    tracing::info!(
        op = "research",
        duration_ms = start.elapsed().as_millis() as u64,
        sample_questions_n = out.sample_questions.len(),
        sources_n = out.sources.len(),
        "research done",
    );

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

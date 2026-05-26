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
    interview_process: String,
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

    // Belt-and-suspenders: append `:online` if missing AND also send the web
    // plugin in the body. Either one alone would enable OpenRouter's web
    // search; sending both makes the intent explicit and survives setting
    // drift (e.g. user pasting a bare model id into settings).
    let model = ensure_online_suffix(&settings.research_model);

    let span = tracing::info_span!(
        "research",
        company = %company_name,
        role,
        model = %model,
    );
    let _enter = span.enter();
    let start = std::time::Instant::now();

    let prompt = prompt_store::get_prompt(db, "research")
        .await?
        .ok_or_else(|| AppError::NotFound("research prompt".into()))?;
    let body = prompt_store::get_current_body_with_schema(db, "research").await?;

    // Wrap user-typed fields in named tags so the model treats them as data,
    // not as instructions. The system prompt is updated to reinforce this.
    let user = format!(
        "<user_input>\n<company_name>{company_name}</company_name>\n<role>{role}</role>\n</user_input>\n\n\
         Produce the research packet now."
    );

    let result = or
        .chat_with_web_search(
            &model,
            vec![ChatMessage::system(body), ChatMessage::user(user)],
            true,
        )
        .await?;
    let cleaned_content = crate::services::llm_safety::check_output("research", &result.content)?;

    let out: AgentOutput = parse_json(&cleaned_content).map_err(|e| {
        tracing::warn!(error = %e, raw_preview = %openrouter::preview(&result.content, 240), "research JSON parse failed");
        AppError::Upstream(format!(
            "research agent returned invalid JSON: {e} — raw: {}",
            openrouter::preview(&result.content, 240)
        ))
    })?;

    if result.fetched_urls.is_empty() {
        tracing::warn!(
            event = "research.no_web_search",
            model = %model,
            "research returned no web-search annotations — model may be answering from training data",
        );
    }

    tracing::info!(
        op = "research",
        duration_ms = start.elapsed().as_millis() as u64,
        sample_questions_n = out.sample_questions.len(),
        sources_n = out.sources.len(),
        fetched_urls_n = result.fetched_urls.len(),
        "research done",
    );

    Ok(ResearchPacket {
        summary: out.summary,
        role_jd: out.role_jd,
        values_signal: out.values_signal,
        interview_process: out.interview_process,
        sample_questions: out.sample_questions,
        sources: out.sources,
        fetched_urls: result.fetched_urls,
        research_prompt_version_id: prompt.current_version_id,
        last_refreshed_at: chrono::Utc::now(),
    })
}

fn ensure_online_suffix(model: &str) -> String {
    if model.ends_with(":online") {
        model.to_string()
    } else {
        format!("{model}:online")
    }
}

#[cfg(test)]
#[path = "research_tests.rs"]
mod tests;

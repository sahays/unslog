//! Tag question texts with canonical category IDs via the cheap `lite_model`.
//! Used during company create / refresh-packet to label agent-generated
//! questions, and on manually pasted questions in the company show page.
//!
//! The classifier sees the canonical category list (id + description) and
//! returns 1–3 IDs per question. Failures fall through to empty tag lists —
//! questions still land, just untagged.

use serde::Deserialize;

use crate::error::AppError;
use crate::models::Category;
use crate::services::openrouter::{parse_json, ChatMessage, OpenRouter};

#[derive(Debug, Deserialize)]
struct ClassificationItem {
    /// Index into the `questions` array we sent.
    question_index: usize,
    /// Category IDs from the canonical list. Returned IDs that aren't in our
    /// pool get filtered out by the caller.
    #[serde(default)]
    categories: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ClassificationOutput {
    #[serde(default)]
    classifications: Vec<ClassificationItem>,
}

/// Classify a batch of question texts. Returns one `Vec<CategoryId>` per
/// input question, in order. On any failure (LLM, parse, etc.) returns
/// empty vectors — caller should treat that as "untagged" and continue.
pub async fn classify_batch(
    or: &OpenRouter,
    lite_model: &str,
    questions: &[String],
    canonical: &[Category],
) -> Vec<Vec<String>> {
    if questions.is_empty() {
        return Vec::new();
    }
    let span = tracing::info_span!(
        "categorize",
        model = lite_model,
        questions_n = questions.len(),
        canonical_n = canonical.len(),
    );
    let _enter = span.enter();
    let start = std::time::Instant::now();

    match try_classify_batch(or, lite_model, questions, canonical).await {
        Ok(tags) => {
            let assigned_total: usize = tags.iter().map(|v| v.len()).sum();
            tracing::info!(
                op = "categorize",
                duration_ms = start.elapsed().as_millis() as u64,
                assigned_total,
                "categorize done",
            );
            tags
        }
        Err(e) => {
            tracing::warn!(error = %e, "categorize failed; questions will land untagged");
            vec![Vec::new(); questions.len()]
        }
    }
}

async fn try_classify_batch(
    or: &OpenRouter,
    lite_model: &str,
    questions: &[String],
    canonical: &[Category],
) -> Result<Vec<Vec<String>>, AppError> {
    let valid_ids: std::collections::HashSet<&str> =
        canonical.iter().map(|c| c.id.as_str()).collect();

    let canonical_block = canonical
        .iter()
        .map(|c| format!("- `{}` ({}): {}", c.id, c.name, c.description))
        .collect::<Vec<_>>()
        .join("\n");

    let questions_block = questions
        .iter()
        .enumerate()
        .map(|(i, q)| format!("{i}. {q}"))
        .collect::<Vec<_>>()
        .join("\n");

    let system = "You are a behavioral-interview competency classifier. Given a list of behavioral interview questions and a list of canonical competency categories, you tag each question with the 1–3 categories it most directly probes. Be precise — pick only categories that the question actually tests, not loosely-related ones.";

    let user = format!(
        r#"<canonical_categories>
{canonical_block}
</canonical_categories>

<questions>
{questions_block}
</questions>

For each question, return the canonical category IDs (lowercase snake_case,
exactly as listed above) that the question tests. 1–3 IDs per question.
Return only the JSON object — no other text, no fences.

Schema:
{{
  "classifications": [
    {{ "question_index": 0, "categories": ["ownership", "prioritization"] }},
    {{ "question_index": 1, "categories": ["communication"] }}
  ]
}}"#
    );

    let raw = or
        .chat(
            lite_model,
            vec![ChatMessage::system(system), ChatMessage::user(user)],
            true,
        )
        .await?;

    let parsed: ClassificationOutput = parse_json(&raw)?;

    let mut out: Vec<Vec<String>> = vec![Vec::new(); questions.len()];
    for item in parsed.classifications {
        if item.question_index >= out.len() {
            continue;
        }
        let cleaned: Vec<String> = item
            .categories
            .into_iter()
            .map(|c| c.trim().to_lowercase())
            .filter(|c| valid_ids.contains(c.as_str()))
            .collect();
        out[item.question_index] = cleaned;
    }
    Ok(out)
}

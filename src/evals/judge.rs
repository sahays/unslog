//! LLM-judge layer — grok-4.3 (via OpenRouter) scores each candidate output
//! against the gold reference on 5 dimensions per target. The judge prompt
//! is structured so the reply is `{ "dimensions": [{name, score, justification}, ...] }`
//! and we parse strictly.
//!
//! Cross-model independence on purpose: the chat/critique stack uses
//! Sonnet/GPT today, the judge uses grok. Same-model judging biases toward
//! "looks fine to me."

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::evals::gold::{ChatTurnGold, CompanyGold, StoryGold};
use crate::evals::Target;
use crate::models::ChatRole;
use crate::services::openrouter::{self, ChatMessage, LlmClient};

pub const JUDGE_MODEL: &str = "x-ai/grok-4.3";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimensionScore {
    pub dimension: String,
    pub score: u8,
    pub justification: String,
}

#[derive(Debug, Clone)]
pub struct JudgeResult {
    pub dimensions: Vec<DimensionScore>,
    /// Mean of all dimension scores (None if the judge returned no scorable
    /// dimensions — usually because the request failed).
    pub aggregate: Option<f32>,
    pub error: Option<String>,
}

#[derive(Deserialize)]
struct JudgeRaw {
    dimensions: Vec<DimensionScore>,
}

pub async fn judge_story_summary(
    client: &dyn LlmClient,
    gold: &StoryGold,
    candidate_body_md: &str,
) -> JudgeResult {
    let dims = STORY_SUMMARY_DIMENSIONS;
    let reference = render_story_body(&gold.body);
    let user = format!(
        "Competency: {competency}\n\n\
         === REFERENCE bullet summary (already accepted as good) ===\n{reference}\n\n\
         === CANDIDATE bullet summary (under evaluation) ===\n{candidate}\n",
        competency = gold.competency_name,
        reference = reference,
        candidate = candidate_body_md,
    );
    run_judge(client, Target::StorySummary, dims, &user).await
}

pub async fn judge_story_chat(client: &dyn LlmClient, gold: &StoryGold) -> JudgeResult {
    let dims = STORY_CHAT_DIMENSIONS;
    let chat = render_chat(&gold.chat);
    let user = format!(
        "Competency: {competency}\nCoach mode: {mode}\n\n\
         === Full coach/candidate chat transcript ===\n{chat}\n\n\
         Grade the coach's behavior across this whole transcript on the dimensions \
         below. There is no reference transcript — you're evaluating the coach in \
         absolute terms against what good probing looks like.",
        competency = gold.competency_name,
        mode = gold.mode.as_str(),
        chat = chat,
    );
    run_judge(client, Target::StoryChat, dims, &user).await
}

pub async fn judge_company(
    client: &dyn LlmClient,
    gold: &CompanyGold,
    candidate_packet_md: &str,
) -> JudgeResult {
    let dims = COMPANY_DIMENSIONS;
    let reference = render_packet(gold);
    let user = format!(
        "Company: {name}\nRole (verbatim): {role}\nCanonical role: {canonical}\n\n\
         === REFERENCE research packet (already accepted as good) ===\n{reference}\n\n\
         === CANDIDATE research packet (under evaluation) ===\n{candidate}\n",
        name = gold.name,
        role = gold.role,
        canonical = gold.canonical_role,
        reference = reference,
        candidate = candidate_packet_md,
    );
    run_judge(client, Target::Company, dims, &user).await
}

/// Score the gold entry against itself — used in `score` where the only
/// "candidate" we have is the gold itself. The judge still produces useful
/// per-dimension absolute scores (and the gold is by definition a 5/5
/// reference on most dimensions, so anything < 5 surfaces a weak gold
/// entry the user should consider pruning).
pub async fn judge_story_summary_self(client: &dyn LlmClient, gold: &StoryGold) -> JudgeResult {
    let candidate = render_story_body(&gold.body);
    judge_story_summary(client, gold, &candidate).await
}

pub async fn judge_company_self(client: &dyn LlmClient, gold: &CompanyGold) -> JudgeResult {
    let candidate = render_packet(gold);
    judge_company(client, gold, &candidate).await
}

// ── Dimension catalogues ───────────────────────────────────────────────

struct DimensionSpec {
    name: &'static str,
    description: &'static str,
}

const STORY_SUMMARY_DIMENSIONS: &[DimensionSpec] = &[
    DimensionSpec {
        name: "STAR+ coverage",
        description: "All of Situation, Task, Action, Result, Reflection present and non-trivial.",
    },
    DimensionSpec {
        name: "Specificity",
        description: "Concrete numbers, names, durations, technologies — not vague claims like 'we improved efficiency'.",
    },
    DimensionSpec {
        name: "First-person ownership",
        description: "'I' clearly owns the action; 'we' is rare and explicitly scoped to the team.",
    },
    DimensionSpec {
        name: "Brevity",
        description: "Tight, scannable bullets; no prose padding or repetition.",
    },
    DimensionSpec {
        name: "Faithfulness to chat",
        description: "Doesn't invent details (numbers, outcomes, roles) that aren't supported by the chat history.",
    },
];

const STORY_CHAT_DIMENSIONS: &[DimensionSpec] = &[
    DimensionSpec {
        name: "Focused turns",
        description: "Each coach turn asks ONE clear question, not a compound multi-question dump.",
    },
    DimensionSpec {
        name: "Builds on prior",
        description: "Each probe builds on the candidate's last answer, not a generic next-step.",
    },
    DimensionSpec {
        name: "Non-redundant",
        description: "Doesn't re-ask things the candidate already answered; no circling on covered ground.",
    },
    DimensionSpec {
        name: "Specificity-pulling",
        description: "Probes pull for who/when/numbers/decisions, not vibes or feelings.",
    },
    DimensionSpec {
        name: "Mode adherence",
        description: "Strict mode = no volunteered wording; Collaborative = offers 2–3 options only when the candidate explicitly asks for help.",
    },
];

const COMPANY_DIMENSIONS: &[DimensionSpec] = &[
    DimensionSpec {
        name: "Company-specificity",
        description: "Would this packet apply equally to any company in this role bucket? Red flag if yes.",
    },
    DimensionSpec {
        name: "Role accuracy",
        description: "role_jd matches the named role AND the company's known scope.",
    },
    DimensionSpec {
        name: "Values concreteness",
        description: "Values backed by examples / behaviors, not abstract platitudes.",
    },
    DimensionSpec {
        name: "Question relevance",
        description: "Sample questions are clearly tailored to THIS company's culture and THIS role, not generic behavioral.",
    },
    DimensionSpec {
        name: "Source faithfulness",
        description: "Cited URLs are real, on-domain, and actually support the claim they're attached to.",
    },
];

// ── Internals ──────────────────────────────────────────────────────────

async fn run_judge(
    client: &dyn LlmClient,
    target: Target,
    dims: &'static [DimensionSpec],
    user_prompt: &str,
) -> JudgeResult {
    let system = build_system_prompt(target, dims);
    let messages = vec![
        ChatMessage::system(system),
        ChatMessage::user(user_prompt.to_string()),
    ];
    match call_and_parse(client, messages).await {
        Ok(scores) => {
            let agg = if scores.is_empty() {
                None
            } else {
                let total: u32 = scores.iter().map(|d| d.score as u32).sum();
                Some(total as f32 / scores.len() as f32)
            };
            JudgeResult {
                dimensions: scores,
                aggregate: agg,
                error: None,
            }
        }
        Err(e) => JudgeResult {
            dimensions: Vec::new(),
            aggregate: None,
            error: Some(format!("{e}")),
        },
    }
}

async fn call_and_parse(
    client: &dyn LlmClient,
    messages: Vec<ChatMessage>,
) -> Result<Vec<DimensionScore>, AppError> {
    let raw = client.chat(JUDGE_MODEL, messages, true).await?;
    let stripped = openrouter::unwrap_fenced_json(&raw);
    let parsed: JudgeRaw = openrouter::parse_json(stripped)?;
    Ok(parsed.dimensions)
}

fn build_system_prompt(target: Target, dims: &[DimensionSpec]) -> String {
    let mut s = String::new();
    s.push_str("You are an evaluator scoring outputs from a behavioral-interview coaching app. ");
    s.push_str("You score on the dimensions below, each 1–5 (1 = unusable, 5 = excellent). ");
    s.push_str(
        "Be strict: a 5 means the output cannot be meaningfully improved on that dimension.\n\n",
    );
    s.push_str(&format!("Target: `{}`\n\n", target.as_str()));
    s.push_str("Dimensions:\n");
    for d in dims {
        s.push_str(&format!("- **{}** — {}\n", d.name, d.description));
    }
    s.push_str("\nReturn a single JSON object with this exact shape:\n");
    s.push_str("{\n  \"dimensions\": [\n");
    s.push_str("    { \"dimension\": \"<name verbatim>\", \"score\": <1-5>, \"justification\": \"<one short sentence>\" }\n");
    s.push_str("  ]\n}\n");
    s.push_str("Use the dimension names verbatim. Include every dimension in the order given. Do not wrap in code fences. Output JSON and nothing else.\n");
    s
}

fn render_story_body(body: &crate::models::StoryBody) -> String {
    let mut s = String::new();
    for (label, section) in [
        ("Situation", &body.situation),
        ("Task", &body.task),
        ("Action", &body.action),
        ("Result", &body.result),
        ("Reflection", &body.reflection),
    ] {
        s.push_str(&format!("## {label}\n"));
        if section.is_empty() {
            s.push_str("_(empty)_\n");
        } else {
            for b in section {
                s.push_str(&format!("- {b}\n"));
            }
        }
        s.push('\n');
    }
    s
}

fn render_chat(chat: &[ChatTurnGold]) -> String {
    let mut s = String::new();
    for turn in chat {
        let who = match turn.role {
            ChatRole::Assistant => "COACH",
            ChatRole::User => "CANDIDATE",
        };
        s.push_str(&format!("[{who}] {}\n\n", turn.content));
    }
    s
}

fn render_packet(g: &CompanyGold) -> String {
    let p = &g.packet;
    let mut s = String::new();
    s.push_str(&format!("## Summary\n{}\n\n", p.summary));
    s.push_str(&format!("## Role JD\n{}\n\n", p.role_jd));
    s.push_str(&format!("## Values signal\n{}\n\n", p.values_signal));
    if !p.interview_process.is_empty() {
        s.push_str(&format!(
            "## Interview process\n{}\n\n",
            p.interview_process
        ));
    }
    s.push_str("## Sample questions\n");
    for q in &p.sample_questions {
        s.push_str(&format!("- {q}\n"));
    }
    s.push('\n');
    s.push_str("## Sources\n");
    for src in &p.sources {
        s.push_str(&format!(
            "- [{}]({}) — {}\n",
            src.title, src.url, src.snippet
        ));
    }
    s
}

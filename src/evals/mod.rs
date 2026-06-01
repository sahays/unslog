//! Eval harness — local-only quality + regression tooling for the LLM-driven
//! features. Three subcommands surface through `src/bin/eval.rs`:
//!
//! * `extract` — pull "accepted" outputs from Postgres (completed stories
//!   with their current StoryVersion; companies with a research packet)
//!   and write them under `data/evals/gold/` as the reference set.
//! * `score` — for each gold entry, run the cheap rubric checks
//!   (machine-checkable, no LLM) and the grok-4.3 LLM-judge (5-dimension
//!   rubric per target), then write a Markdown report under
//!   `data/evals/reports/<iso-ts>/`.
//! * `regression` — replay each gold entry's input through a baseline vs
//!   candidate version of a named prompt (using the prompt store's
//!   snapshot infra) and diff the outputs, optionally asking the judge
//!   which is better.
//!
//! Eval *targets* are the units the suite grades:
//!
//! * `story_summary` — the final STAR+ bullet body
//! * `story_chat` — the coach's probing behavior across a chat
//! * `company` — the research packet
//!
//! Practice is out of scope (no per-critique "user accepted" signal).
//!
//! Everything is local. No network in extract or rubric. Judge + regression
//! call OpenRouter (grok-4.3) and depend on `OPENROUTER_API_KEY` in `.env`.

pub mod adversarial;
pub mod extract;
pub mod gold;
pub mod judge;
pub mod regression;
pub mod report;
pub mod rubric;
pub mod score;

/// All eval targets. Used by the CLI to enumerate / filter what to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    StorySummary,
    StoryChat,
    Company,
    Adversarial,
}

impl Target {
    pub const ALL: [Target; 4] = [
        Target::StorySummary,
        Target::StoryChat,
        Target::Company,
        Target::Adversarial,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Target::StorySummary => "story_summary",
            Target::StoryChat => "story_chat",
            Target::Company => "company",
            Target::Adversarial => "adversarial",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "story_summary" => Some(Target::StorySummary),
            "story_chat" => Some(Target::StoryChat),
            "company" => Some(Target::Company),
            "adversarial" => Some(Target::Adversarial),
            _ => None,
        }
    }
}

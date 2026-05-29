//! Machine-checkable structural rules — the cheap floor that runs on every
//! `eval score` invocation, no LLM calls. Each target has its own check
//! function returning a list of failure strings; an empty list = passed.
//!
//! These checks intentionally don't grade quality. They catch refusals,
//! schema breaks, length collapses, and prompt-leakage — the stuff that
//! shows up as obviously-wrong output rather than subtly-worse output.

use crate::evals::gold::{ChatTurnGold, CompanyGold, StoryGold};
use crate::models::{ChatRole, StoryBody};

const REFUSAL_MARKERS: &[&str] = &[
    "i cannot help",
    "i can't help",
    "i'm sorry, but i can't",
    "as an ai",
    "as a large language model",
    "i am not able to",
];

const LEAKAGE_MARKERS: &[&str] = &[
    "<competency>",
    "</competency>",
    "<thinking>",
    "system:",
    "<system>",
];

/// Lead-word markers that indicate a third-person / passive bullet — the
/// failure mode we want to catch. Detecting these as a negative signal is
/// way more robust than a positive whitelist of "good" verbs (which is
/// always too narrow). The LLM judge owns the deeper first-person-ownership
/// scoring; this is just the cheap floor.
const PASSIVE_BULLET_LEADS: &[&str] = &[
    "we ",
    "we'",
    "our team",
    "the team",
    "there was",
    "there were",
    "it was decided",
    "everyone ",
    "everybody ",
    "people ",
];

/// Words that can legitimately open a behavioral-style question even though
/// the sentence as a whole ends with `.` rather than `?`. Lets the rubric
/// accept "Tell me about a time…", "Walk me through…", etc. as well-formed.
const IMPERATIVE_QUESTION_LEADS: &[&str] = &[
    "tell ",
    "describe ",
    "walk ",
    "explain ",
    "share ",
    "give ",
    "talk ",
];

#[derive(Debug, Clone)]
pub struct RubricResult {
    pub target_id: String,
    pub target_label: String,
    pub failures: Vec<String>,
}

impl RubricResult {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

// ── story_summary ──────────────────────────────────────────────────────

pub fn check_story_summary(gold: &StoryGold) -> RubricResult {
    let mut failures = Vec::new();
    let body = &gold.body;
    let total_bullets = total_bullets(body);
    if total_bullets == 0 {
        failures.push("body is empty — no bullets in any STAR+ section".into());
    }
    if total_bullets > 30 {
        failures.push(format!(
            "body has {total_bullets} bullets — should sprawl past ~20"
        ));
    }
    if body.situation.is_empty() {
        failures.push("STAR+ Situation section is empty".into());
    }
    if body.action.is_empty() {
        failures.push("STAR+ Action section is empty".into());
    }
    if body.result.is_empty() {
        failures.push("STAR+ Result section is empty".into());
    }
    let total_words = count_words(&body_text(body));
    if total_words > 400 {
        failures.push(format!(
            "summary length {total_words}w exceeds 400-word soft cap"
        ));
    }
    let blob = body_text(body).to_lowercase();
    check_markers(&blob, &mut failures);
    let passive_n = count_passive_action_bullets(&body.action);
    if passive_n > 0 {
        failures.push(format!(
            "{passive_n}/{} Action bullet(s) lead with passive/third-person framing (we, the team, there was, …)",
            body.action.len()
        ));
    }
    RubricResult {
        target_id: gold.id.clone(),
        target_label: format!("{} — v{}", gold.competency_name, gold.current_version_n),
        failures,
    }
}

// ── story_chat ─────────────────────────────────────────────────────────

pub fn check_story_chat(gold: &StoryGold) -> RubricResult {
    let mut failures = Vec::new();
    let assistant_turns: Vec<&ChatTurnGold> = gold
        .chat
        .iter()
        .filter(|t| matches!(t.role, ChatRole::Assistant))
        .collect();
    let user_turns: Vec<&ChatTurnGold> = gold
        .chat
        .iter()
        .filter(|t| matches!(t.role, ChatRole::User))
        .collect();
    if assistant_turns.is_empty() {
        failures.push("chat has no assistant turns — coach never spoke".into());
    }
    if user_turns.is_empty() {
        failures.push("chat has no user turns — story never built".into());
    }
    // Coach should ask questions — at least 80% of assistant turns should
    // contain '?'. Lower than that suggests the coach is monologuing rather
    // than probing.
    if !assistant_turns.is_empty() {
        let with_q = assistant_turns
            .iter()
            .filter(|t| t.content.contains('?'))
            .count();
        let ratio = with_q as f32 / assistant_turns.len() as f32;
        if ratio < 0.8 {
            failures.push(format!(
                "only {:.0}% of coach turns contain a question — coach should be probing",
                ratio * 100.0
            ));
        }
    }
    // Exact-duplicate assistant turns are a cheap floor for "circling" — the
    // real check lives in the LLM judge (paraphrased re-asks), but identical
    // text is a clear bug worth catching here.
    let mut seen = std::collections::HashSet::new();
    for turn in &assistant_turns {
        if !seen.insert(turn.content.trim()) {
            failures.push("coach repeated an identical turn — verbatim circling".into());
            break;
        }
    }
    // Refusal / leakage markers.
    let blob: String = assistant_turns
        .iter()
        .map(|t| t.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    check_markers(&blob, &mut failures);

    RubricResult {
        target_id: gold.id.clone(),
        target_label: format!(
            "{} — {} turns ({} coach / {} user)",
            gold.competency_name,
            gold.chat.len(),
            assistant_turns.len(),
            user_turns.len()
        ),
        failures,
    }
}

// ── company ────────────────────────────────────────────────────────────

pub fn check_company(gold: &CompanyGold) -> RubricResult {
    let mut failures = Vec::new();
    let p = &gold.packet;
    if p.summary.trim().is_empty() {
        failures.push("research packet summary is empty".into());
    }
    if p.role_jd.trim().is_empty() {
        failures.push("research packet role_jd is empty".into());
    }
    if p.values_signal.trim().is_empty() {
        failures.push("research packet values_signal is empty".into());
    }
    let summary_words = count_words(&p.summary);
    if summary_words < 100 {
        failures.push(format!(
            "summary is only {summary_words}w — too shallow (≥100 expected)"
        ));
    }
    if summary_words > 600 {
        failures.push(format!(
            "summary is {summary_words}w — runaway (≤600 expected)"
        ));
    }
    let jd_words = count_words(&p.role_jd);
    if jd_words < 50 {
        failures.push(format!(
            "role_jd is only {jd_words}w — too shallow (≥50 expected)"
        ));
    }
    let n_q = p.sample_questions.len();
    if !(4..=12).contains(&n_q) {
        failures.push(format!("{n_q} sample questions — expected 4–12"));
    }
    for q in &p.sample_questions {
        let trimmed = q.trim();
        if !is_well_formed_question(trimmed) {
            failures.push(format!(
                "sample question is malformed (doesn't end with '?' and doesn't start with an imperative): {trimmed:?}"
            ));
            break;
        }
    }
    if p.fetched_urls.is_empty() {
        failures.push(
            "no fetched_urls — OpenRouter web plugin didn't run (model may be hallucinating)"
                .into(),
        );
    }
    for src in &p.sources {
        if !is_plausible_url(&src.url) {
            failures.push(format!("source has implausible url: {:?}", src.url));
            break;
        }
    }
    let blob = format!(
        "{}\n{}\n{}\n{}",
        p.summary, p.role_jd, p.values_signal, p.interview_process
    )
    .to_lowercase();
    check_markers(&blob, &mut failures);

    RubricResult {
        target_id: gold.id.clone(),
        target_label: format!("{} — {}", gold.name, gold.role),
        failures,
    }
}

// ── helpers ────────────────────────────────────────────────────────────

fn total_bullets(b: &StoryBody) -> usize {
    b.situation.len() + b.task.len() + b.action.len() + b.result.len() + b.reflection.len()
}

fn body_text(b: &StoryBody) -> String {
    let mut out = String::new();
    for section in [&b.situation, &b.task, &b.action, &b.result, &b.reflection] {
        for bullet in section {
            out.push_str(bullet);
            out.push('\n');
        }
    }
    out
}

fn count_words(s: &str) -> usize {
    s.split_whitespace().count()
}

fn check_markers(haystack: &str, failures: &mut Vec<String>) {
    for marker in REFUSAL_MARKERS {
        if haystack.contains(marker) {
            failures.push(format!("contains refusal marker: {marker:?}"));
            break;
        }
    }
    for marker in LEAKAGE_MARKERS {
        if haystack.contains(marker) {
            failures.push(format!("contains prompt-leakage marker: {marker:?}"));
            break;
        }
    }
}

fn count_passive_action_bullets(bullets: &[String]) -> usize {
    bullets
        .iter()
        .filter(|b| {
            let lower = b.trim_start().to_lowercase();
            PASSIVE_BULLET_LEADS.iter().any(|m| lower.starts_with(m))
        })
        .count()
}

fn is_well_formed_question(q: &str) -> bool {
    let trimmed = q.trim();
    if trimmed.ends_with('?') {
        return true;
    }
    let lower = trimmed.to_lowercase();
    IMPERATIVE_QUESTION_LEADS
        .iter()
        .any(|m| lower.starts_with(m))
}

fn is_plausible_url(s: &str) -> bool {
    let s = s.trim();
    (s.starts_with("http://") || s.starts_with("https://")) && s.contains('.') && !s.contains(' ')
}

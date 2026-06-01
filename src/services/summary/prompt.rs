//! User-message rendering for the summary prompt. Pure functions over the
//! domain models — no I/O, no DB. Split out of `summary.rs` to keep that
//! file under the 300 LOC cap.

use crate::models::{Company, Evaluation, Summary};

pub(super) fn render_user_message(
    company: &Company,
    evals: &[Evaluation],
    priors: &[Summary],
) -> String {
    let packet = company_packet(company);
    let priors_block = render_priors_block(priors);
    let evals_block = render_evals_block(evals);

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

Write the debrief now."#
    )
}

fn render_priors_block(priors: &[Summary]) -> String {
    if priors.is_empty() {
        return "(no prior session summaries for this company yet)".to_string();
    }
    priors
        .iter()
        .enumerate()
        .map(|(i, s)| render_prior(i + 1, s))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

fn render_prior(idx_from_one: usize, s: &Summary) -> String {
    let weak = optional_block("recurring_weaknesses", &s.recurring_weaknesses);
    let blind = optional_block("blind_spots", &s.blind_spots);
    format!(
        "Session -{} ({}):\n{}{}{}",
        idx_from_one,
        s.created_at.format("%Y-%m-%d"),
        s.narrative,
        weak,
        blind,
    )
}

fn optional_block(label: &str, items: &[String]) -> String {
    if items.is_empty() {
        String::new()
    } else {
        format!("\n{}: {}", label, items.join("; "))
    }
}

fn render_evals_block(evals: &[Evaluation]) -> String {
    if evals.is_empty() {
        return "(no questions answered in this session)".to_string();
    }
    evals
        .iter()
        .enumerate()
        .map(|(i, e)| render_eval(i + 1, e))
        .collect::<Vec<_>>()
        .join("\n\n===\n\n")
}

fn render_eval(idx: usize, e: &Evaluation) -> String {
    let attempts = if e.attempts.is_empty() {
        "(no attempts — question was skipped)".to_string()
    } else {
        e.attempts
            .iter()
            .map(render_attempt)
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    format!("Q{idx}. {}\n\n{attempts}", e.question_text)
}

fn render_attempt(a: &crate::models::Attempt) -> String {
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

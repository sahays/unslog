//! Markdown report writer. Each `score` run drops a timestamped folder
//! under `<data_dir>/evals/reports/` with one file per target plus a
//! top-level `summary.md`. Reports are local-only — never committed.

use std::fmt::Write as _;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::evals::judge::JudgeResult;
use crate::evals::rubric::RubricResult;
use crate::evals::Target;

pub struct ReportDir {
    pub path: PathBuf,
}

pub fn open_report_dir(data_dir: &str) -> Result<ReportDir> {
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let path = PathBuf::from(data_dir).join("evals/reports").join(ts);
    std::fs::create_dir_all(&path).with_context(|| format!("mkdir -p {}", path.display()))?;
    Ok(ReportDir { path })
}

/// One eval target's full result set — rubric + (optional) judge per entry.
pub struct TargetReport {
    pub target: Target,
    pub entries: Vec<EntryReport>,
}

pub struct EntryReport {
    pub rubric: RubricResult,
    pub judge: Option<JudgeResult>,
}

pub fn write_target_report(dir: &ReportDir, report: &TargetReport) -> Result<PathBuf> {
    let path = dir.path.join(format!("{}.md", report.target.as_str()));
    let body = render_target(report);
    std::fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

pub fn write_summary(dir: &ReportDir, all: &[TargetReport]) -> Result<PathBuf> {
    let path = dir.path.join("summary.md");
    let mut s = String::new();
    let _ = writeln!(s, "# Eval run — {}", dir.path.display());
    let _ = writeln!(s);
    let _ = writeln!(s, "| Target | Entries | Rubric passed | Judge avg |");
    let _ = writeln!(s, "|---|---|---|---|");
    for tr in all {
        let n = tr.entries.len();
        let passed = tr.entries.iter().filter(|e| e.rubric.passed()).count();
        let judge_avg = judge_avg(tr);
        let judge_cell = judge_avg
            .map(|x| format!("{x:.2}"))
            .unwrap_or_else(|| "—".into());
        let _ = writeln!(
            s,
            "| `{}` | {} | {}/{} | {} |",
            tr.target.as_str(),
            n,
            passed,
            n,
            judge_cell
        );
    }
    let _ = writeln!(s);
    let _ = writeln!(s, "Per-target details in the sibling `<target>.md` files.");
    std::fs::write(&path, s).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

fn judge_avg(tr: &TargetReport) -> Option<f32> {
    let scores: Vec<f32> = tr
        .entries
        .iter()
        .filter_map(|e| e.judge.as_ref())
        .filter_map(|j| j.aggregate)
        .collect();
    if scores.is_empty() {
        None
    } else {
        Some(scores.iter().sum::<f32>() / scores.len() as f32)
    }
}

fn render_target(report: &TargetReport) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "# {} — {} entries",
        report.target.as_str(),
        report.entries.len()
    );
    let _ = writeln!(s);
    let total = report.entries.len();
    let passed = report.entries.iter().filter(|e| e.rubric.passed()).count();
    let _ = writeln!(s, "**Rubric:** {passed}/{total} passed.");
    if let Some(avg) = judge_avg(report) {
        let _ = writeln!(
            s,
            "**Judge (grok-4.3):** average {avg:.2}/5 across {} entries.",
            report
                .entries
                .iter()
                .filter(|e| e.judge.as_ref().and_then(|j| j.aggregate).is_some())
                .count()
        );
    }
    let _ = writeln!(s);
    for entry in &report.entries {
        render_entry(&mut s, entry);
    }
    s
}

fn render_entry(out: &mut String, entry: &EntryReport) {
    let _ = writeln!(
        out,
        "## {} — `{}`",
        entry.rubric.target_label, entry.rubric.target_id
    );
    if entry.rubric.passed() {
        let _ = writeln!(out, "- **Rubric:** ✅ all checks passed");
    } else {
        let _ = writeln!(
            out,
            "- **Rubric:** ❌ {} failure(s)",
            entry.rubric.failures.len()
        );
        for f in &entry.rubric.failures {
            let _ = writeln!(out, "    - {f}");
        }
    }
    if let Some(j) = &entry.judge {
        if let Some(agg) = j.aggregate {
            let _ = writeln!(out, "- **Judge:** aggregate {agg:.2}/5");
        }
        for s in &j.dimensions {
            let _ = writeln!(
                out,
                "    - **{}**: {}/5 — {}",
                s.dimension, s.score, s.justification
            );
        }
        if let Some(err) = &j.error {
            let _ = writeln!(out, "    - ⚠️ judge error: {err}");
        }
    }
    let _ = writeln!(out);
}

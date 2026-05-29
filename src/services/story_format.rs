//! Format helpers that turn a `StoryBody` into prompt-input text. Shared by
//! the refine-kickoff and spoken-monologue services since both feed the
//! bullets back into an LLM call with the same `Section:\n- bullet\n` shape.

use crate::models::StoryBody;

/// Render a `StoryBody` as plain-text bullets grouped by section. Empty
/// sections are emitted as `Section: (empty)` so the model can see the gap
/// instead of silently skipping it.
pub fn bullets(body: &StoryBody) -> String {
    let mut out = String::new();
    out.push_str(&section("Situation", &body.situation));
    out.push('\n');
    out.push_str(&section("Task", &body.task));
    out.push('\n');
    out.push_str(&section("Action", &body.action));
    out.push('\n');
    out.push_str(&section("Result", &body.result));
    out.push('\n');
    out.push_str(&section("Reflection", &body.reflection));
    out
}

fn section(label: &str, items: &[String]) -> String {
    if items.is_empty() {
        format!("{label}: (empty)\n")
    } else {
        let lines = items
            .iter()
            .map(|b| format!("- {b}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("{label}:\n{lines}\n")
    }
}

#[cfg(test)]
#[path = "story_format_tests.rs"]
mod tests;

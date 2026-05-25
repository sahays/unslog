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
mod tests {
    use super::*;

    #[test]
    fn case_all_empty_sections() {
        let out = bullets(&StoryBody::default());
        assert!(out.contains("Situation: (empty)"));
        assert!(out.contains("Task: (empty)"));
        assert!(out.contains("Action: (empty)"));
        assert!(out.contains("Result: (empty)"));
        assert!(out.contains("Reflection: (empty)"));
    }

    #[test]
    fn case_partial_sections() {
        let body = StoryBody {
            situation: vec!["s1".to_string()],
            ..StoryBody::default()
        };
        let out = bullets(&body);
        assert!(out.contains("Situation:\n- s1"));
        assert!(out.contains("Task: (empty)"));
    }

    #[test]
    fn case_full_body() {
        let body = StoryBody {
            situation: vec!["s1".to_string()],
            task: vec!["t1".to_string(), "t2".to_string()],
            action: vec!["a1".to_string()],
            result: vec!["r1".to_string()],
            reflection: vec!["x1".to_string()],
        };
        let out = bullets(&body);
        assert!(out.contains("- s1"));
        assert!(out.contains("- t1"));
        assert!(out.contains("- t2"));
        assert!(out.contains("- a1"));
        assert!(out.contains("- r1"));
        assert!(out.contains("- x1"));
        assert!(!out.contains("(empty)"));
    }
}

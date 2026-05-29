use super::*;

/// LLM input integrity: missing sections must surface as the explicit
/// `(empty)` sentinel — otherwise the model silently skips gaps and
/// the candidate never gets prompted on what's missing.
#[test]
fn empty_sections_render_as_explicit_empty_sentinel_for_llm() {
    let out = bullets(&StoryBody::default());
    assert!(out.contains("Situation: (empty)"));
    assert!(out.contains("Task: (empty)"));
    assert!(out.contains("Action: (empty)"));
    assert!(out.contains("Result: (empty)"));
    assert!(out.contains("Reflection: (empty)"));
}

/// LLM input integrity: populated bullets render with the `- ` prefix
/// and never collapse with the `(empty)` sentinel.
#[test]
fn populated_sections_render_as_dashed_bullets_without_empty_sentinel() {
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

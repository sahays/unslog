use super::*;
use crate::models::StoryBody;

fn turn(role: ChatRole, content: &str) -> ChatTurn {
    ChatTurn {
        role,
        content: content.to_string(),
        ts: chrono::Utc::now(),
    }
}

// ── strip_lock_in_token ──────────────────────────────────────────────

#[test]
fn case_token_absent() {
    let (cleaned, locked) = strip_lock_in_token("just a normal reply");
    assert_eq!(cleaned, "just a normal reply");
    assert!(!locked);
}

#[test]
fn case_token_present_at_end() {
    let (cleaned, locked) = strip_lock_in_token("text <<LOCK_IN>>");
    assert_eq!(cleaned, "text");
    assert!(locked);
}

#[test]
fn case_token_present_with_whitespace() {
    let (cleaned, locked) = strip_lock_in_token("text\n\n<<LOCK_IN>>\n");
    assert_eq!(cleaned, "text");
    assert!(locked);
}

#[test]
fn case_token_only() {
    let (cleaned, locked) = strip_lock_in_token("<<LOCK_IN>>");
    assert_eq!(cleaned, "");
    assert!(locked);
}

#[test]
fn case_token_in_middle() {
    // current behavior: replace leaves a double space; trim only touches
    // edges.
    let (cleaned, locked) = strip_lock_in_token("a <<LOCK_IN>> b");
    assert_eq!(cleaned, "a  b");
    assert!(locked);
}

#[test]
fn case_multiple_tokens() {
    let (cleaned, locked) = strip_lock_in_token("a <<LOCK_IN>> b <<LOCK_IN>>");
    assert_eq!(cleaned, "a  b");
    assert!(locked);
}

// ── render_transcript ────────────────────────────────────────────────

#[test]
fn case_empty_chat() {
    assert_eq!(render_transcript(&[]), "");
}

#[test]
fn case_single_user_turn() {
    let chat = vec![turn(ChatRole::User, "hello")];
    assert_eq!(render_transcript(&chat), "CANDIDATE:\nhello");
}

#[test]
fn case_alternating_turns() {
    let chat = vec![
        turn(ChatRole::Assistant, "ask"),
        turn(ChatRole::User, "answer"),
    ];
    let out = render_transcript(&chat);
    assert_eq!(out, "COACH:\nask\n\n---\n\nCANDIDATE:\nanswer");
}

// ── render_bullets ───────────────────────────────────────────────────

#[test]
fn case_all_empty_sections() {
    let body = StoryBody::default();
    let out = render_bullets(&body);
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
    let out = render_bullets(&body);
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
    let out = render_bullets(&body);
    assert!(out.contains("- s1"));
    assert!(out.contains("- t1"));
    assert!(out.contains("- t2"));
    assert!(out.contains("- a1"));
    assert!(out.contains("- r1"));
    assert!(out.contains("- x1"));
    assert!(!out.contains("(empty)"));
}

use super::*;

fn turn(role: ChatRole, content: &str) -> ChatTurn {
    ChatTurn {
        role,
        content: content.to_string(),
        ts: chrono::Utc::now(),
    }
}

/// LLM context integrity: empty chat must produce an empty string so
/// the wrapping `<chat_transcript>` block stays well-formed with no
/// stray `CANDIDATE:` / `COACH:` header.
#[test]
fn render_empty_chat_produces_empty_string_so_transcript_block_stays_clean() {
    assert_eq!(render(&[]), "");
}

/// LLM context integrity: the exact label vocabulary
/// (`CANDIDATE`/`COACH`) and the `\n\n---\n\n` separator are what the
/// coach prompt is trained against. Any drift here changes how the
/// model parses turn boundaries.
#[test]
fn render_uses_candidate_and_coach_labels_with_hr_separator_for_prompt() {
    let chat = vec![
        turn(ChatRole::Assistant, "ask"),
        turn(ChatRole::User, "answer"),
    ];
    assert_eq!(render(&chat), "COACH:\nask\n\n---\n\nCANDIDATE:\nanswer");
}

//! Shared transcript-rendering for chat turns.
//!
//! Stories' `run_generate` / `continue_chat` and Pitches' `pitch_lockin`
//! all need the same wire format: `CANDIDATE:`/`COACH:` labelled blocks
//! joined by `\n\n---\n\n`. Each used to inline its own copy of the same
//! `chat.iter().map(...).collect().join(...)` snippet. This module owns it.

use crate::models::{ChatRole, ChatTurn};

/// Join `turns` into a single string with `CANDIDATE:` / `COACH:` labels,
/// separated by a markdown-style horizontal rule. Suitable for embedding
/// inside a `<chat_transcript>` block in a prompt.
pub fn render(turns: &[ChatTurn]) -> String {
    turns
        .iter()
        .map(format_turn)
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

fn format_turn(t: &ChatTurn) -> String {
    let label = match t.role {
        ChatRole::User => "CANDIDATE",
        ChatRole::Assistant => "COACH",
    };
    format!("{label}:\n{}", t.content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(role: ChatRole, content: &str) -> ChatTurn {
        ChatTurn {
            role,
            content: content.to_string(),
            ts: chrono::Utc::now(),
        }
    }

    #[test]
    fn case_empty_input_yields_empty_string() {
        assert_eq!(render(&[]), "");
    }

    #[test]
    fn case_labels_and_separator_match_legacy_format() {
        let chat = vec![
            turn(ChatRole::Assistant, "ask"),
            turn(ChatRole::User, "answer"),
        ];
        assert_eq!(render(&chat), "COACH:\nask\n\n---\n\nCANDIDATE:\nanswer");
    }
}

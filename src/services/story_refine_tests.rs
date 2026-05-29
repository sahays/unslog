use super::last_n_in_order;
use crate::models::{ChatRole, ChatTurn};

fn turn(content: &str) -> ChatTurn {
    ChatTurn {
        role: ChatRole::User,
        content: content.to_string(),
        ts: chrono::Utc::now(),
    }
}

/// LLM context integrity: the refine probe is fed the tail of the
/// chat history. The helper must (a) return the correct tail when
/// `n < len` and (b) saturate (not panic / wrap) when `n > len`.
#[test]
fn last_n_returns_correct_tail_and_does_not_underflow_when_n_exceeds_len() {
    let v = vec![turn("a"), turn("b"), turn("c"), turn("d"), turn("e")];
    let tail = last_n_in_order(&v, 2);
    assert_eq!(
        tail.iter().map(|t| t.content.as_str()).collect::<Vec<_>>(),
        vec!["d", "e"],
    );
    assert_eq!(last_n_in_order(&v, 10).len(), 5);
}

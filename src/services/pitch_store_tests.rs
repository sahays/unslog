use super::{check_chat_capacity, MAX_CHAT_TURNS};
use crate::error::AppError;

#[test]
fn capacity_ok_at_just_below_cap() {
    assert!(check_chat_capacity(MAX_CHAT_TURNS - 1).is_ok());
}

#[test]
fn capacity_rejects_at_cap_with_actionable_message() {
    let err = check_chat_capacity(MAX_CHAT_TURNS).expect_err("should reject at cap");
    match err {
        AppError::BadRequest(msg) => {
            assert!(msg.contains(&MAX_CHAT_TURNS.to_string()), "message: {msg}");
            assert!(msg.to_lowercase().contains("lock"), "message: {msg}");
        }
        other => panic!("expected BadRequest, got {other:?}"),
    }
}

#[test]
fn capacity_rejects_over_cap() {
    assert!(check_chat_capacity(MAX_CHAT_TURNS + 1).is_err());
    assert!(check_chat_capacity(MAX_CHAT_TURNS * 2).is_err());
}

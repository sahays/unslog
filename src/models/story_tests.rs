//! Tests defending the form→storage→form round-trip on `Difficulty` and
//! the fallback path that protects the LLM prompt-name dispatch against
//! invalid form input.

use super::*;

/// Trust-boundary fallback: unknown form values must not crash and must
/// default to a known difficulty — otherwise a tampered form picks a
/// nonexistent prompt name and the chat handler explodes downstream.
#[test]
fn difficulty_unknown_form_value_defaults_to_strict_not_panic() {
    assert_eq!(Difficulty::from_form("medium"), Difficulty::Strict);
    assert_eq!(Difficulty::from_form(""), Difficulty::Strict);
}

/// LLM input integrity: each difficulty must route to its own prompt
/// name so the wrong system prompt is never sent to the coach.
#[test]
fn difficulty_routes_to_correct_prompt_name_for_llm() {
    assert_eq!(Difficulty::Strict.prompt_name(), "story_chat");
    assert_eq!(
        Difficulty::Collaborative.prompt_name(),
        "story_chat_collaborative"
    );
}

/// Form↔storage round-trip: `as_str` writes the value into the form, the
/// next submit parses it back. If the round-trip ever asymmetrizes, the
/// dropdown silently changes the user's choice between requests.
#[test]
fn difficulty_round_trips_form_to_storage_to_form() {
    assert_eq!(
        Difficulty::from_form(Difficulty::Strict.as_str()),
        Difficulty::Strict
    );
    assert_eq!(
        Difficulty::from_form(Difficulty::Collaborative.as_str()),
        Difficulty::Collaborative
    );
}

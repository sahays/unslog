use super::*;

#[test]
fn case_difficulty_default_is_strict() {
    assert_eq!(Difficulty::default(), Difficulty::Strict);
}

#[test]
fn case_difficulty_from_form_collaborative() {
    assert_eq!(
        Difficulty::from_form("collaborative"),
        Difficulty::Collaborative
    );
}

#[test]
fn case_difficulty_from_form_strict() {
    assert_eq!(Difficulty::from_form("strict"), Difficulty::Strict);
}

#[test]
fn case_difficulty_from_form_unknown_falls_back_to_strict() {
    assert_eq!(Difficulty::from_form("medium"), Difficulty::Strict);
    assert_eq!(Difficulty::from_form(""), Difficulty::Strict);
}

#[test]
fn case_difficulty_prompt_name_routes_by_mode() {
    assert_eq!(Difficulty::Strict.prompt_name(), "story_chat");
    assert_eq!(
        Difficulty::Collaborative.prompt_name(),
        "story_chat_collaborative"
    );
}

#[test]
fn case_difficulty_as_str_round_trips_through_from_form() {
    assert_eq!(
        Difficulty::from_form(Difficulty::Strict.as_str()),
        Difficulty::Strict
    );
    assert_eq!(
        Difficulty::from_form(Difficulty::Collaborative.as_str()),
        Difficulty::Collaborative
    );
}

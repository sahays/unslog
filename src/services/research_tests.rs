use super::*;

#[test]
fn case_ensure_online_suffix_appends_when_missing() {
    assert_eq!(
        ensure_online_suffix("google/gemini-2.5-pro"),
        "google/gemini-2.5-pro:online"
    );
}

#[test]
fn case_ensure_online_suffix_idempotent_when_present() {
    assert_eq!(
        ensure_online_suffix("google/gemini-2.5-pro:online"),
        "google/gemini-2.5-pro:online"
    );
}

#[test]
fn case_ensure_online_suffix_does_not_double_up() {
    let once = ensure_online_suffix("anthropic/claude-3.5-sonnet");
    let twice = ensure_online_suffix(&once);
    assert_eq!(once, twice);
}

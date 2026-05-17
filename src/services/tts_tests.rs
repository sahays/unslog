use super::*;

#[test]
fn case_build_accent_en_us() {
    assert_eq!(
        build_accent_instructions("en-US"),
        "Speak clearly with a American English accent."
    );
}

#[test]
fn case_build_accent_en_gb() {
    assert_eq!(
        build_accent_instructions("en-GB"),
        "Speak clearly with a British English accent."
    );
}

#[test]
fn case_build_accent_en_in() {
    assert_eq!(
        build_accent_instructions("en-IN"),
        "Speak clearly with a Indian English accent."
    );
}

#[test]
fn case_build_accent_en_au() {
    assert_eq!(
        build_accent_instructions("en-AU"),
        "Speak clearly with a Australian English accent."
    );
}

#[test]
fn case_build_accent_empty_returns_empty() {
    assert!(build_accent_instructions("").is_empty());
}

#[test]
fn case_build_accent_unknown_returns_empty() {
    assert!(build_accent_instructions("fr-FR").is_empty());
    assert!(build_accent_instructions("not-a-locale").is_empty());
}

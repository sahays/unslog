use super::*;

fn fixture_settings() -> Settings {
    Settings {
        id: "default".into(),
        critique_model: "anthropic/claude-sonnet-4".into(),
        research_model: "google/gemini-2.5-pro".into(),
        stt_model: "openai/whisper-1".into(),
        tts_model: "openai/gpt-4o-mini-tts".into(),
        tts_voice: "alloy".into(),
        tts_language: "en-US".into(),
        tts_speed: Some(1.25),
        lite_model: "google/gemini-2.5-flash".into(),
        updated_at: chrono::Utc::now(),
    }
}

fn fixture_input() -> StartInput {
    StartInput {
        role: Role::ProductManager,
        anchor_company_id: "co-anchor".into(),
        selected_company_ids: vec!["co-anchor".into(), "co-other".into()],
    }
}

fn fixture_curated() -> CuratorOutput {
    CuratorOutput {
        question_ids: vec!["q-1".into(), "q-2".into()],
        focus_line: "focus on cross-functional conflict".into(),
    }
}

const TEST_OWNER: &str = "usrtest1";

/// Prompt-snapshot integrity: the version IDs handed in MUST land in
/// `prompt_snapshot` verbatim. If they don't, every critique +
/// summary on this session will resolve against the wrong (later)
/// prompt version and the snapshot-on-use guarantee breaks silently.
#[test]
fn build_session_captures_prompt_version_ids_verbatim_into_snapshot() {
    let session = build_session(
        TEST_OWNER,
        fixture_input(),
        &fixture_settings(),
        "critique-version-abc",
        "summary-version-xyz",
        fixture_curated(),
    );
    assert_eq!(session.prompt_snapshot.critique, "critique-version-abc");
    assert_eq!(session.prompt_snapshot.summary, "summary-version-xyz");
}

/// Model-snapshot integrity: the per-session model choices must
/// mirror the current Settings document at start time, so that a
/// later Settings edit doesn't retroactively change which model
/// transcribed / critiqued the session.
#[test]
fn build_session_mirrors_current_settings_into_model_snapshot_for_replay_integrity() {
    let settings = fixture_settings();
    let session = build_session(
        TEST_OWNER,
        fixture_input(),
        &settings,
        "c-v",
        "s-v",
        fixture_curated(),
    );
    assert_eq!(session.model_snapshot.critique, settings.critique_model);
    assert_eq!(session.model_snapshot.research, settings.research_model);
    assert_eq!(session.model_snapshot.stt, settings.stt_model);
    assert_eq!(session.model_snapshot.tts, settings.tts_model);
    assert_eq!(session.model_snapshot.tts_voice, settings.tts_voice);
    assert_eq!(session.model_snapshot.tts_language, settings.tts_language);
    assert_eq!(session.model_snapshot.tts_speed, settings.tts_speed);
    assert_eq!(session.model_snapshot.lite, settings.lite_model);
}

/// Lifecycle defaults: a freshly-built session must start in `Active`
/// with no ended timestamp and no preloaded question — otherwise the
/// show-page renders pre-populated state that no one asked for.
#[test]
fn build_session_starts_active_with_no_question_loaded_and_unique_id() {
    let s1 = build_session(
        TEST_OWNER,
        fixture_input(),
        &fixture_settings(),
        "c",
        "s",
        fixture_curated(),
    );
    let s2 = build_session(
        TEST_OWNER,
        fixture_input(),
        &fixture_settings(),
        "c",
        "s",
        fixture_curated(),
    );
    assert_eq!(s1.status, SessionStatus::Active);
    assert!(s1.ended_at.is_none());
    assert!(s1.current_question_id.is_none());
    assert!(s1.current_question_text.is_none());
    assert!(s1.current_question_audio_path.is_none());
    assert!(!s1.voice_critique_enabled);
    assert!(!s1.id.is_empty());
    assert_ne!(s1.id, s2.id, "session IDs must be distinct");
    // Owner-scoping invariant — sessions inserted without an owner could
    // be read by the wrong user (or by no one).
    assert_eq!(s1.owner_id, TEST_OWNER);
}

use super::*;
use crate::models::{ModelSnapshot, PromptSnapshot, Role, SessionStatus};
use mockall::predicate;

fn fixture_session() -> Session {
    Session {
        id: "sess-1".into(),
        owner_id: "usrmaster".into(),
        company_id: "co-1".into(),
        role: Role::ProductManager,
        selected_company_ids: vec!["co-1".into()],
        curated_question_ids: vec![],
        focus_line: String::new(),
        started_at: chrono::Utc::now(),
        ended_at: None,
        status: SessionStatus::Active,
        model_snapshot: ModelSnapshot {
            stt: "stt".into(),
            tts: "tts".into(),
            critique: "c".into(),
            research: "r".into(),
            tts_voice: "alloy".into(),
            tts_language: String::new(),
            tts_speed: Some(1.0),
            lite: "l".into(),
        },
        prompt_snapshot: PromptSnapshot {
            critique: "p-c".into(),
            summary: "p-s".into(),
        },
        voice_critique_enabled: false,
        current_question_id: None,
        current_question_text: None,
        current_question_audio_path: None,
    }
}

fn eval_with_attempts(n: u32) -> Evaluation {
    let attempts: Vec<Attempt> = (1..=n)
        .map(|i| Attempt {
            attempt_n: i,
            answer_audio_path: None,
            answer_transcript: format!("a{i}"),
            critique: None,
            critique_audio_path: None,
            created_at: chrono::Utc::now(),
        })
        .collect();
    Evaluation {
        id: "eval-x".into(),
        owner_id: "usrmaster".into(),
        session_id: "sess-1".into(),
        company_id: "co-1".into(),
        question_id: "q-1".into(),
        question_text: "tell me a time...".into(),
        attempts,
    }
}

#[tokio::test]
async fn case_load_or_create_returns_existing() {
    // Existing eval with 2 attempts → next attempt_n should be 3.
    let mut deps = MockEvalSource::new();
    deps.expect_find()
        .with(
            predicate::eq("usrmaster"),
            predicate::eq("sess-1"),
            predicate::eq("q-1"),
        )
        .times(1)
        .returning(|_, _, _| Ok(Some(eval_with_attempts(2))));

    let session = fixture_session();
    let (eval, attempt_n) = load_or_create(&deps, &session, "q-1", "ignored")
        .await
        .expect("ok");
    assert_eq!(eval.attempts.len(), 2);
    assert_eq!(attempt_n, 3);
}

#[tokio::test]
async fn case_load_or_create_creates_when_missing() {
    // Nothing on disk → returns a fresh Evaluation, attempts empty,
    // attempt_n == 1.
    let mut deps = MockEvalSource::new();
    deps.expect_find().times(1).returning(|_, _, _| Ok(None));

    let session = fixture_session();
    let (eval, attempt_n) = load_or_create(&deps, &session, "q-new", "fresh question")
        .await
        .expect("ok");
    assert!(eval.attempts.is_empty());
    assert_eq!(attempt_n, 1);
    assert_eq!(eval.session_id, "sess-1");
    assert_eq!(eval.question_id, "q-new");
    assert_eq!(eval.question_text, "fresh question");
}

#[tokio::test]
async fn case_load_or_create_propagates_error() {
    let mut deps = MockEvalSource::new();
    deps.expect_find()
        .times(1)
        .returning(|_, _, _| Err(AppError::Other(anyhow::anyhow!("db down"))));

    let session = fixture_session();
    let err = load_or_create(&deps, &session, "q-1", "x")
        .await
        .expect_err("expected error");
    assert!(matches!(err, AppError::Other(_)));
}

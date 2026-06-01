use super::*;
use crate::models::{ModelSnapshot, PromptSnapshot, Role, SessionStatus};
use crate::services::openrouter::MockLlmClient;
use mockall::predicate;

fn fixture_session() -> Session {
    Session {
        id: "sess-1".to_string(),
        company_id: "co-1".to_string(),
        role: Role::ProductManager,
        selected_company_ids: vec!["co-1".to_string()],
        curated_question_ids: vec![],
        focus_line: String::new(),
        started_at: chrono::Utc::now(),
        ended_at: None,
        status: SessionStatus::Active,
        model_snapshot: ModelSnapshot {
            stt: "stt".into(),
            tts: "tts".into(),
            critique: "critique-model".into(),
            research: "research".into(),
            tts_voice: "alloy".into(),
            tts_language: String::new(),
            tts_speed: Some(1.0),
            lite: "lite".into(),
        },
        prompt_snapshot: PromptSnapshot {
            critique: "prompt-v-7".into(),
            summary: "prompt-v-8".into(),
        },
        voice_critique_enabled: false,
        current_question_id: None,
        current_question_text: None,
        current_question_audio_path: None,
    }
}

fn fixture_company() -> Company {
    let now = chrono::Utc::now();
    Company {
        id: "co-1".into(),
        owner_id: "usrmaster".into(),
        name: "Acme".into(),
        role: "PM".into(),
        canonical_role: Role::ProductManager,
        research_packet: None,
        is_public: false,
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
async fn case_run_parses_critique_json() {
    let mut deps = MockCritiqueDeps::new();
    deps.expect_get_critique_prompt_body()
        .with(predicate::eq("prompt-v-7"))
        .times(1)
        .returning(|_| Ok("be a critic".to_string()));
    deps.expect_get_book_text()
        .times(1)
        .returning(|| Ok(Arc::new("BOOK".to_string())));

    let mut llm = MockLlmClient::new();
    llm.expect_chat()
        .with(
            predicate::eq("critique-model"),
            predicate::always(),
            predicate::eq(true),
        )
        .times(1)
        .returning(|_, _, _| {
            Ok(r#"```json
{
  "scores": {"specificity": 4, "role_clarity": 3, "star_plus_structure": 4, "pitfalls_avoided": 3, "company_fit": 5},
  "narrative": "solid attempt",
  "citations": [],
  "improved_vs_prior": ""
}
```"#
                .to_string())
        });

    let session = fixture_session();
    let company = fixture_company();
    let out = run(
        &deps,
        &llm,
        &session,
        &company,
        "tell me about a time...",
        "i did the thing",
        &[],
        &[],
    )
    .await
    .expect("run should succeed");

    assert_eq!(out.scores.specificity, 4);
    assert_eq!(out.scores.company_fit, Some(5));
    assert_eq!(out.narrative, "solid attempt");
}

#[tokio::test]
async fn case_run_propagates_llm_error() {
    let mut deps = MockCritiqueDeps::new();
    deps.expect_get_critique_prompt_body()
        .returning(|_| Ok("p".to_string()));
    deps.expect_get_book_text()
        .returning(|| Ok(Arc::new("b".to_string())));

    let mut llm = MockLlmClient::new();
    llm.expect_chat()
        .returning(|_, _, _| Err(AppError::Upstream("boom".into())));

    let session = fixture_session();
    let company = fixture_company();
    let err = run(&deps, &llm, &session, &company, "q", "a", &[], &[])
        .await
        .expect_err("expected upstream error");
    assert!(matches!(err, AppError::Upstream(_)));
}

#[tokio::test]
async fn case_run_returns_parse_error_on_garbage() {
    let mut deps = MockCritiqueDeps::new();
    deps.expect_get_critique_prompt_body()
        .returning(|_| Ok("p".to_string()));
    deps.expect_get_book_text()
        .returning(|| Ok(Arc::new("b".to_string())));

    let mut llm = MockLlmClient::new();
    llm.expect_chat()
        .returning(|_, _, _| Ok("not json at all".to_string()));

    let session = fixture_session();
    let company = fixture_company();
    let err = run(&deps, &llm, &session, &company, "q", "a", &[], &[])
        .await
        .expect_err("expected parse error");
    let msg = err.to_string();
    // The function maps the parse error into an Upstream that contains a
    // preview of the raw garbage — pin that contract.
    assert!(msg.contains("invalid JSON"), "got: {msg}");
}

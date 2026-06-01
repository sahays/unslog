use super::*;
use crate::models::{ModelSnapshot, PromptSnapshot, Role, SessionStatus};
use crate::services::openrouter::MockLlmClient;
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
        status: SessionStatus::Ended,
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
            critique: "p-c".into(),
            summary: "p-s".into(),
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

fn existing_summary() -> Summary {
    Summary {
        id: "sum-existing".into(),
        owner_id: "usrmaster".into(),
        session_id: "sess-1".into(),
        company_id: "co-1".into(),
        narrative: "already done".into(),
        strengths: vec![],
        recurring_weaknesses: vec![],
        blind_spots: vec![],
        company_fit_signal: String::new(),
        created_at: chrono::Utc::now(),
    }
}

#[tokio::test]
async fn case_generate_skips_when_already_present() {
    // Idempotency: an existing summary short-circuits the LLM and the
    // insert. Pin those .times(0) call counts.
    let mut deps = MockSummaryDeps::new();
    deps.expect_find_existing_summary()
        .with(predicate::eq("usrmaster"), predicate::eq("sess-1"))
        .times(1)
        .returning(|_, _| Ok(Some(existing_summary())));
    deps.expect_insert_summary().times(0);

    let mut llm = MockLlmClient::new();
    llm.expect_chat().times(0);

    let session = fixture_session();
    let company = fixture_company();
    let out = generate_and_save(&deps, &llm, &session, &company)
        .await
        .expect("ok");
    assert_eq!(out.id, "sum-existing");
    assert_eq!(out.narrative, "already done");
}

#[tokio::test]
async fn case_generate_calls_llm_when_absent() {
    let mut deps = MockSummaryDeps::new();
    deps.expect_find_existing_summary()
        .times(1)
        .returning(|_, _| Ok(None));
    deps.expect_get_summary_prompt_body()
        .with(predicate::eq("p-s"))
        .times(1)
        .returning(|_| Ok("be a coach".into()));
    deps.expect_find_session_evaluations()
        .times(1)
        .returning(|_, _| Ok(vec![]));
    deps.expect_recent_company_summaries_for_prompt()
        .times(1)
        .returning(|_, _, _, _| Ok(vec![]));
    deps.expect_insert_summary().times(1).returning(|_| Ok(()));

    let mut llm = MockLlmClient::new();
    llm.expect_chat().times(1).returning(|_, _, _| {
        Ok(r#"{
  "narrative": "did fine",
  "strengths": ["clear stories"],
  "recurring_weaknesses": [],
  "blind_spots": [],
  "company_fit_signal": "neutral"
}"#
        .to_string())
    });

    let session = fixture_session();
    let company = fixture_company();
    let out = generate_and_save(&deps, &llm, &session, &company)
        .await
        .expect("ok");
    assert_eq!(out.session_id, "sess-1");
    assert_eq!(out.narrative, "did fine");
    assert_eq!(out.strengths, vec!["clear stories".to_string()]);
}

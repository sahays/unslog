//! Unit tests for the rubric checks. These run without Mongo or any LLM —
//! they construct gold fixtures in-memory and assert what passes/fails.

use super::*;

use crate::models::{ChatRole, Difficulty, ResearchPacket, ResearchSource, StoryBody};

fn make_story(body: StoryBody, chat: Vec<ChatTurnGold>) -> StoryGold {
    StoryGold {
        id: "s-1".into(),
        competency_id: "c-1".into(),
        competency_name: "Conflict".into(),
        mode: Difficulty::Strict,
        chat,
        current_version_n: 1,
        body,
    }
}

fn full_body() -> StoryBody {
    StoryBody {
        situation: vec!["Joined a team mid-quarter with shifting priorities.".into()],
        task: vec!["Take ownership of the migration plan.".into()],
        action: vec![
            "Drove a series of design reviews with three stakeholders.".into(),
            "Built a phased rollout doc that the leads signed off on.".into(),
        ],
        result: vec!["Shipped on schedule with 0 P0 incidents.".into()],
        reflection: vec!["I would scope the second phase tighter next time.".into()],
    }
}

fn good_packet() -> ResearchPacket {
    // Note: summary ≥ 100 words and role_jd ≥ 50 words to exercise the
    // length floors in the rubric. Sample questions all end with '?'.
    ResearchPacket {
        summary: "Acme is a 500-person logistics-software company headquartered in Austin, Texas. \
        Public since 2019, recently posted record quarterly revenue on the back of \
        enterprise tier upsells. Engineering culture emphasizes ownership and \
        operational rigor; the SE organization sits inside Customer Outcomes and reports to \
        the CTO via a VP of Field Engineering. Recent leadership hires include \
        a new VP of Enterprise Sales and a Director of Solutions Architecture, both \
        with strong enterprise backgrounds. The company is hiring across SE, AE, and \
        CSM functions, with an explicit bias toward candidates who have sold to \
        enterprise IT and can speak fluently about deployment patterns, integration \
        risk, and total cost of ownership across multi-quarter rollouts."
            .into(),
        role_jd: "Solutions Architect on the Field Engineering team. Partners closely with \
        Account Executives to discover requirements, scope proofs of concept, and present \
        technical architectures to enterprise prospects. Expected to be hands-on with the \
        Acme platform, write reference architectures, and own the technical close. \
        Reports into the VP of Field Engineering and works cross-functionally with \
        Product and Customer Success."
            .into(),
        values_signal: "Ownership, low ego, operational rigor. Recent blog posts highlight \
        post-mortem culture and a clear escalation ladder."
            .into(),
        interview_process: "Recruiter screen, hiring manager, panel including AE partner, \
        and an on-site case study with a take-home deck."
            .into(),
        sample_questions: vec![
            "Can you walk me through a recent enterprise POC you led end to end?".into(),
            "Tell me about a time a customer's stated requirement wasn't the real need?".into(),
            "How do you handle pushback from a vendor's engineering team during a POC?".into(),
            "Describe a time you missed a milestone in a customer engagement?".into(),
        ],
        sources: vec![ResearchSource {
            url: "https://acme.example.com/about".into(),
            title: "About Acme".into(),
            snippet: "Logistics platform for enterprise shippers.".into(),
        }],
        fetched_urls: vec!["https://acme.example.com/about".into()],
        research_prompt_version_id: "p-1".into(),
        last_refreshed_at: chrono::Utc::now(),
    }
}

fn good_company() -> CompanyGold {
    CompanyGold {
        id: "co-1".into(),
        name: "Acme".into(),
        role: "Solutions Architect".into(),
        canonical_role: "solutions_architect".into(),
        packet: good_packet(),
    }
}

#[test]
fn case_story_summary_passes_full_body() {
    let res = check_story_summary(&make_story(full_body(), Vec::new()));
    assert!(res.passed(), "expected pass; failures = {:?}", res.failures);
}

#[test]
fn case_story_summary_flags_empty_body() {
    let res = check_story_summary(&make_story(StoryBody::default(), Vec::new()));
    assert!(!res.passed());
    assert!(res.failures.iter().any(|f| f.contains("empty")));
}

#[test]
fn case_story_summary_flags_refusal_marker() {
    let mut body = full_body();
    body.reflection.push("I cannot help with that.".into());
    let res = check_story_summary(&make_story(body, Vec::new()));
    assert!(res.failures.iter().any(|f| f.contains("refusal")));
}

#[test]
fn case_story_summary_flags_oversize() {
    let mut body = full_body();
    // 410-word reflection bullet — single bullet, > 400-word cap.
    body.reflection.push("blah ".repeat(410).trim().to_string());
    let res = check_story_summary(&make_story(body, Vec::new()));
    assert!(res.failures.iter().any(|f| f.contains("400-word")));
}

#[test]
fn case_story_chat_flags_no_assistant_turns() {
    let chat = vec![ChatTurnGold {
        role: ChatRole::User,
        content: "I want to talk about a conflict at work.".into(),
    }];
    let res = check_story_chat(&make_story(full_body(), chat));
    assert!(res.failures.iter().any(|f| f.contains("no assistant")));
}

#[test]
fn case_story_chat_flags_verbatim_circling() {
    let chat = vec![
        ChatTurnGold {
            role: ChatRole::Assistant,
            content: "Who was involved and what was the friction?".into(),
        },
        ChatTurnGold {
            role: ChatRole::User,
            content: "My PM and I disagreed on scope.".into(),
        },
        ChatTurnGold {
            role: ChatRole::Assistant,
            content: "Who was involved and what was the friction?".into(),
        },
    ];
    let res = check_story_chat(&make_story(full_body(), chat));
    assert!(res.failures.iter().any(|f| f.contains("circling")));
}

#[test]
fn case_company_passes_good_packet() {
    let res = check_company(&good_company());
    assert!(res.passed(), "expected pass; failures = {:?}", res.failures);
}

#[test]
fn case_company_flags_too_few_questions() {
    let mut gold = good_company();
    gold.packet.sample_questions.truncate(2);
    let res = check_company(&gold);
    assert!(res.failures.iter().any(|f| f.contains("sample questions")));
}

#[test]
fn case_company_flags_empty_fetched_urls() {
    let mut gold = good_company();
    gold.packet.fetched_urls.clear();
    let res = check_company(&gold);
    assert!(res.failures.iter().any(|f| f.contains("fetched_urls")));
}

#[test]
fn case_company_flags_implausible_source_url() {
    let mut gold = good_company();
    gold.packet.sources[0].url = "not-a-url".into();
    let res = check_company(&gold);
    assert!(res.failures.iter().any(|f| f.contains("implausible")));
}

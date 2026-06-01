//! Shared fixtures + helpers for the cross-tenant isolation suite.
//!
//! `setup_two_users` provisions Alice and Bob on a fresh per-test database
//! (`#[sqlx::test]` runs migrations before this is called). Both ids
//! satisfy the `^usr[a-z0-9]{6}$` CHECK on `users.id`. Every test that
//! needs catalog-shaped fixtures (`Company`, `Session`, `Story`, …) builds
//! them via the small helpers in this file so the call sites stay short.

use chrono::Utc;
use sqlx::PgPool;

use unslog::models::{
    Asset, AssetKind, Company, Evaluation, ExtractionStatus, JournalEntry, ModelSnapshot,
    PromptSnapshot, Question, QuestionSource, Role, Session, SessionStatus, Story, StoryStatus,
    Summary, User, UserTier,
};
use unslog::services::{
    asset_store, company_store, evaluations as eval_store, journal_store, questions, sessions,
    story_store, summary,
};

pub struct Users {
    pub alice: User,
    pub bob: User,
}

/// Insert two non-master users (Alice + Bob) and return them.
pub async fn setup_two_users(pool: &PgPool) -> Users {
    let alice = insert_user(pool, "usralice", "alice").await;
    let bob = insert_user(pool, "usrbobbob", "bob").await;
    Users { alice, bob }
}

async fn insert_user(pool: &PgPool, id: &str, label: &str) -> User {
    // The user id CHECK is `^usr[a-z0-9]{6}$` (so trim/pad to 9 chars).
    let id9 = ensure_user_id_shape(id);
    let user = User {
        id: id9.clone(),
        // The hash never gets verified in these tests — the value just
        // needs to satisfy the UNIQUE index on `code_hash`, so we make it
        // a function of the id.
        code_hash: format!("$argon2id$v=19$m=19456,t=2,p=1$xx${id9}"),
        code_hint: "xxxx…xx".into(),
        label: label.to_string(),
        tier: UserTier::Pro,
        is_master: false,
        activated_at: None,
        expires_at: None,
        last_seen_at: None,
        revoked_at: None,
        invited_by: None,
        created_at: Utc::now(),
    };
    unslog::services::user_store::insert(pool, &user)
        .await
        .expect("insert user");
    user
}

fn ensure_user_id_shape(id: &str) -> String {
    assert!(id.starts_with("usr"), "id must start with 'usr'");
    let suffix = &id[3..];
    let mut s: String = suffix
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    while s.len() < 6 {
        s.push('0');
    }
    s.truncate(6);
    format!("usr{s}")
}

// ── Company helpers ──────────────────────────────────────────────────────

pub async fn insert_company(pool: &PgPool, owner: &User, name: &str) -> Company {
    let c = Company::new(
        owner.id.clone(),
        name.to_string(),
        "Engineer".into(),
        Role::SoftwareEngineer,
    );
    company_store::insert(pool, &c)
        .await
        .expect("insert company");
    c
}

// ── Journal helpers ──────────────────────────────────────────────────────

pub async fn insert_journal(pool: &PgPool, owner: &User, title: &str) -> JournalEntry {
    journal_store::create(pool, &owner.id, title.to_string(), "body".into())
        .await
        .expect("create journal")
}

// ── Asset helpers ────────────────────────────────────────────────────────

pub async fn insert_resume(pool: &PgPool, owner: &User, name: &str) -> Asset {
    let mut a = Asset::new(
        owner.id.clone(),
        name.to_string(),
        AssetKind::Resume,
        "resume.pdf".into(),
        format!("data/assets/originals/{}.pdf", &owner.id),
    );
    a.extraction_status = ExtractionStatus::Ok;
    a.primary = true;
    asset_store::insert(pool, &a).await.expect("insert resume");
    a
}

/// Book assets must be owned by the master user (migration 0003 trigger).
pub async fn insert_master_book(pool: &PgPool) -> Asset {
    let mut a = Asset::new(
        unslog::services::master_seed::MASTER_ID.to_string(),
        "the book".to_string(),
        AssetKind::Book,
        "book.pdf".into(),
        "data/assets/originals/book.pdf".into(),
    );
    a.extraction_status = ExtractionStatus::Ok;
    a.primary = true;
    asset_store::insert(pool, &a).await.expect("insert book");
    a
}

// ── Session helpers ──────────────────────────────────────────────────────

pub async fn insert_session(pool: &PgPool, owner: &User, company: &Company) -> Session {
    let now = Utc::now();
    let session = Session {
        id: Session::new_id(),
        owner_id: owner.id.clone(),
        company_id: company.id.clone(),
        role: Role::SoftwareEngineer,
        selected_company_ids: vec![company.id.clone()],
        curated_question_ids: vec![],
        focus_line: String::new(),
        started_at: now,
        ended_at: None,
        status: SessionStatus::Active,
        model_snapshot: ModelSnapshot {
            stt: "stt".into(),
            tts: "tts".into(),
            critique: "crit".into(),
            research: "research".into(),
            tts_voice: String::new(),
            tts_language: String::new(),
            tts_speed: None,
            lite: "lite".into(),
        },
        prompt_snapshot: PromptSnapshot {
            critique: "prv000001".into(),
            summary: "prv000002".into(),
        },
        voice_critique_enabled: false,
        current_question_id: None,
        current_question_text: None,
        current_question_audio_path: None,
    };
    sessions::insert(pool, &session)
        .await
        .expect("insert session");
    session
}

// ── Question helpers ─────────────────────────────────────────────────────

pub async fn insert_question(
    pool: &PgPool,
    owner: &User,
    company_id: Option<String>,
    text: &str,
) -> Question {
    let n = questions::append(
        pool,
        &owner.id,
        vec![text.to_string()],
        QuestionSource::Uploaded,
        Role::SoftwareEngineer,
        company_id.clone(),
        |_t| vec![],
    )
    .await
    .expect("append question");
    assert_eq!(n, 1);
    // Re-fetch the just-inserted question — owner-scoped read. Pool
    // includes role-only (NULL company_id) automatically; for
    // company-tagged questions pass the company id so the
    // `company_id = ANY($3)` clause matches.
    let company_ids: Vec<String> = company_id.into_iter().collect();
    questions::list_for_pool(pool, &owner.id, Role::SoftwareEngineer, &company_ids)
        .await
        .expect("list pool")
        .into_iter()
        .find(|q| q.text == text)
        .expect("question round-trips")
}

// ── Evaluation helpers ───────────────────────────────────────────────────

pub async fn insert_evaluation(
    pool: &PgPool,
    owner: &User,
    session: &Session,
    question: &Question,
) -> Evaluation {
    let eval = Evaluation::new(
        owner.id.clone(),
        session.id.clone(),
        session.company_id.clone(),
        question.id.clone(),
        question.text.clone(),
    );
    let ctx = eval_store::PgEvalSource { pool };
    use unslog::services::evaluations::EvalSource;
    ctx.upsert(&eval).await.expect("insert evaluation");
    eval
}

// ── Summary helpers ──────────────────────────────────────────────────────

pub async fn insert_summary(
    pool: &PgPool,
    owner: &User,
    session: &Session,
    company: &Company,
) -> Summary {
    let s = Summary {
        id: Summary::new_id(),
        owner_id: owner.id.clone(),
        session_id: session.id.clone(),
        company_id: company.id.clone(),
        narrative: "narr".into(),
        strengths: vec!["s1".into()],
        recurring_weaknesses: vec!["w1".into()],
        blind_spots: vec!["b1".into()],
        company_fit_signal: "fit".into(),
        created_at: Utc::now(),
    };
    let ctx = summary::SummaryCtx { pool };
    use unslog::services::summary::SummaryDeps;
    ctx.insert_summary(&s).await.expect("insert summary");
    s
}

// ── Story helpers ────────────────────────────────────────────────────────

/// Insert a category row needed as the FK target for stories (catalog).
pub async fn ensure_category(pool: &PgPool, id: &str) {
    sqlx::query("INSERT INTO categories (id, name) VALUES ($1, $2) ON CONFLICT (id) DO NOTHING")
        .bind(id)
        .bind(id)
        .execute(pool)
        .await
        .expect("insert category");
}

pub async fn insert_story(pool: &PgPool, owner: &User, competency_id: &str) -> Story {
    ensure_category(pool, competency_id).await;
    let now = Utc::now();
    let story = Story {
        id: Story::new_id(),
        owner_id: owner.id.clone(),
        competency_id: competency_id.to_string(),
        status: StoryStatus::InProgress,
        mode: Default::default(),
        current_version_id: None,
        chat: vec![],
        created_at: now,
        updated_at: now,
    };
    story_store::insert(pool, &story)
        .await
        .expect("insert story");
    story
}

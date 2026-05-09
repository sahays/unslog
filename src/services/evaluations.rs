//! Evaluation row read/write.
//!
//! Each `(session_id, question_id)` pair has at most one Evaluation row, with
//! attempts pushed onto an array. The first attempt inserts; subsequent
//! attempts replace. The orchestration handler used to inline both steps —
//! this module owns the upsert so handlers stay thin.

use std::collections::HashMap;

use async_trait::async_trait;
use futures::TryStreamExt;
use mongodb::Database;

use crate::error::AppError;
use crate::models::{Attempt, Evaluation, Session};

/// Persistence seam for [`load_or_create`] and [`commit_attempt`]. Production
/// impl is the inherent `mongodb::Database`; tests inject a mock generated
/// by `mockall::automock`.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait EvalSource: Send + Sync {
    async fn find(
        &self,
        session_id: &str,
        question_id: &str,
    ) -> Result<Option<Evaluation>, AppError>;

    /// First attempt inserts the row; subsequent attempts replace it.
    async fn upsert(&self, eval: &Evaluation) -> Result<(), AppError>;
}

#[async_trait]
impl EvalSource for Database {
    async fn find(
        &self,
        session_id: &str,
        question_id: &str,
    ) -> Result<Option<Evaluation>, AppError> {
        Ok(self
            .collection::<Evaluation>(Evaluation::COLLECTION)
            .find_one(bson::doc! { "session_id": session_id, "question_id": question_id })
            .await?)
    }

    async fn upsert(&self, eval: &Evaluation) -> Result<(), AppError> {
        let coll = self.collection::<Evaluation>(Evaluation::COLLECTION);
        if eval.attempts.len() == 1 {
            coll.insert_one(eval).await?;
        } else {
            coll.replace_one(bson::doc! { "_id": &eval.id }, eval)
                .await?;
        }
        Ok(())
    }
}

/// Load the eval row for `(session, qid)` or build a fresh one. Returns the
/// row and the attempt number the next push should use.
pub async fn load_or_create(
    deps: &dyn EvalSource,
    session: &Session,
    qid: &str,
    qtext: &str,
) -> Result<(Evaluation, u32), AppError> {
    let eval = deps.find(&session.id, qid).await?.unwrap_or_else(|| {
        Evaluation::new(
            session.id.clone(),
            session.company_id.clone(),
            qid.to_string(),
            qtext.to_string(),
        )
    });
    let attempt_n = (eval.attempts.len() as u32) + 1;
    Ok((eval, attempt_n))
}

/// Push `attempt` onto `eval` and persist. First attempt inserts the row;
/// subsequent attempts replace it.
pub async fn commit_attempt(
    deps: &dyn EvalSource,
    mut eval: Evaluation,
    attempt: Attempt,
) -> Result<(), AppError> {
    eval.attempts.push(attempt);
    deps.upsert(&eval).await?;
    Ok(())
}

/// One aggregate over `evaluations` grouped by `session_id` — replaces N
/// `count_documents` queries when listing many sessions on a page.
pub async fn counts_by_session(
    db: &Database,
    session_ids: &[&str],
) -> Result<HashMap<String, usize>, AppError> {
    if session_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let pipeline = vec![
        bson::doc! { "$match": { "session_id": { "$in": session_ids } } },
        bson::doc! { "$group": { "_id": "$session_id", "n": { "$sum": 1 } } },
    ];
    let docs: Vec<bson::Document> = db
        .collection::<bson::Document>(Evaluation::COLLECTION)
        .aggregate(pipeline)
        .await?
        .try_collect()
        .await?;
    let mut out: HashMap<String, usize> = HashMap::with_capacity(docs.len());
    for d in docs {
        if let Ok(id) = d.get_str("_id") {
            // Aggregate sums come back as int32 or int64 depending on mongo
            // version; accept either.
            let n = d
                .get_i64("n")
                .ok()
                .or_else(|| d.get_i32("n").ok().map(i64::from))
                .unwrap_or(0);
            out.insert(id.to_string(), n.max(0) as usize);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ModelSnapshot, PromptSnapshot, Role, SessionStatus};
    use mockall::predicate;

    fn fixture_session() -> Session {
        Session {
            id: "sess-1".into(),
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
            .with(predicate::eq("sess-1"), predicate::eq("q-1"))
            .times(1)
            .returning(|_, _| Ok(Some(eval_with_attempts(2))));

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
        deps.expect_find().times(1).returning(|_, _| Ok(None));

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
            .returning(|_, _| Err(AppError::Other(anyhow::anyhow!("db down"))));

        let session = fixture_session();
        let err = load_or_create(&deps, &session, "q-1", "x")
            .await
            .expect_err("expected error");
        assert!(matches!(err, AppError::Other(_)));
    }
}

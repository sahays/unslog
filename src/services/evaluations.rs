//! Evaluation row read/write.
//!
//! Each `(session_id, question_id)` pair has at most one Evaluation row, with
//! attempts pushed onto an array. The first attempt inserts; subsequent
//! attempts replace. The orchestration handler used to inline both steps —
//! this module owns the upsert so handlers stay thin.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use futures::TryStreamExt;
use mongodb::options::FindOptions;
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

/// Set of `question_id`s already answered in `session_id`. Used by the
/// session-advance engine to skip questions the user has already worked.
/// Projects only `question_id` so we don't ship the embedded `attempts[]`
/// payload (potentially multi-KB per row) across the wire.
pub async fn answered_question_ids(
    db: &Database,
    session_id: &str,
) -> Result<HashSet<String>, AppError> {
    #[derive(serde::Deserialize)]
    struct QidRow {
        question_id: String,
    }
    let opts = FindOptions::builder()
        .projection(bson::doc! { "question_id": 1 })
        .build();
    let rows: Vec<QidRow> = db
        .collection::<QidRow>(Evaluation::COLLECTION)
        .find(bson::doc! { "session_id": session_id })
        .with_options(opts)
        .await?
        .try_collect()
        .await?;
    Ok(rows.into_iter().map(|r| r.question_id).collect())
}

#[cfg(test)]
#[path = "evaluations_tests.rs"]
mod tests;

//! Per-company question bank helpers.

use mongodb::{Collection, Database};

use crate::error::AppError;
use crate::models::question_bank::{Question, QuestionBank, QuestionSource};

fn coll(db: &Database) -> Collection<QuestionBank> {
    db.collection(QuestionBank::COLLECTION)
}

pub async fn ensure_for(db: &Database, company_id: &str) -> Result<QuestionBank, AppError> {
    let c = coll(db);
    if let Some(existing) = c.find_one(bson::doc! { "company_id": company_id }).await? {
        return Ok(existing);
    }
    let bank = QuestionBank::empty(company_id.to_string());
    c.insert_one(&bank).await?;
    Ok(bank)
}

pub async fn get_for(db: &Database, company_id: &str) -> Result<Option<QuestionBank>, AppError> {
    Ok(coll(db).find_one(bson::doc! { "company_id": company_id }).await?)
}

pub async fn append_questions(
    db: &Database,
    company_id: &str,
    texts: impl IntoIterator<Item = String>,
    source: QuestionSource,
) -> Result<usize, AppError> {
    let _ = ensure_for(db, company_id).await?;
    let new_questions: Vec<Question> = texts
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .map(|t| Question::new(t, source))
        .collect();
    if new_questions.is_empty() {
        return Ok(0);
    }
    let count = new_questions.len();
    let docs = new_questions
        .iter()
        .map(bson::to_bson)
        .collect::<Result<Vec<_>, _>>()?;

    coll(db)
        .update_one(
            bson::doc! { "company_id": company_id },
            bson::doc! { "$push": { "questions": { "$each": docs } } },
        )
        .await?;
    Ok(count)
}

pub async fn delete_question(
    db: &Database,
    company_id: &str,
    question_id: &str,
) -> Result<(), AppError> {
    coll(db)
        .update_one(
            bson::doc! { "company_id": company_id },
            bson::doc! { "$pull": { "questions": { "id": question_id } } },
        )
        .await?;
    Ok(())
}

/// Pick the next question, ignoring any IDs in `seen`.
pub fn pick_next<'a>(bank: &'a QuestionBank, seen: &[String]) -> Option<&'a Question> {
    use rand::seq::SliceRandom;
    let candidates: Vec<&Question> = bank
        .questions
        .iter()
        .filter(|q| !seen.iter().any(|s| s == &q.id))
        .collect();
    candidates.choose(&mut rand::thread_rng()).copied()
}

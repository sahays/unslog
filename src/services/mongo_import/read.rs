//! Read pass: fetch every Mongo collection and pre-mint the new Postgres
//! ids so the write pass can do FK rewrites without a second round-trip.
//!
//! All "interesting" inputs go through `mongodb`'s typed [`Collection<T>`]
//! API. We never log document bodies — too much PII risk in chat history
//! and journal entries.

use anyhow::{Context, Result};
use futures::TryStreamExt;
use mongodb::{Collection, Database};

use crate::models::{
    Asset, Category, Company, Evaluation, JournalEntry, Pitch, PitchVersion, Prompt, PromptVersion,
    Question, Session, Settings, Story, StoryVersion, Summary,
};

use super::id_map::{IdMap, Kind};

/// In-memory snapshot of every Mongo collection plus the FK rewrite map.
/// The write pass consumes this as one borrow so there's no read/write
/// interleaving against either store.
pub struct ReadSet {
    pub categories: Vec<Category>,
    pub companies: Vec<Company>,
    pub questions: Vec<Question>,
    pub sessions: Vec<Session>,
    pub evaluations: Vec<Evaluation>,
    pub summaries: Vec<Summary>,
    pub stories: Vec<Story>,
    pub story_versions: Vec<StoryVersion>,
    pub pitches: Vec<Pitch>,
    pub pitch_versions: Vec<PitchVersion>,
    pub journal_entries: Vec<JournalEntry>,
    pub assets: Vec<Asset>,
    pub prompts: Vec<Prompt>,
    pub prompt_versions: Vec<PromptVersion>,
    pub settings: Option<Settings>,
    pub map: IdMap,
}

/// Read every collection and mint ids for FK rewrites. Catalog ids
/// (categories, pitches) and the master user are recorded as fixed
/// passthroughs so calling sites don't need a special case.
pub async fn read_all(db: &Database) -> Result<ReadSet> {
    let categories = fetch_all::<Category>(db, Category::COLLECTION).await?;
    let companies = fetch_all::<Company>(db, Company::COLLECTION).await?;
    let questions = fetch_all::<Question>(db, Question::COLLECTION).await?;
    let sessions = fetch_all::<Session>(db, Session::COLLECTION).await?;
    let evaluations = fetch_all::<Evaluation>(db, Evaluation::COLLECTION).await?;
    let summaries = fetch_all::<Summary>(db, Summary::COLLECTION).await?;
    let stories = fetch_all::<Story>(db, Story::COLLECTION).await?;
    let story_versions = fetch_all::<StoryVersion>(db, StoryVersion::COLLECTION).await?;
    let pitches = fetch_all::<Pitch>(db, Pitch::COLLECTION).await?;
    let pitch_versions = fetch_all::<PitchVersion>(db, PitchVersion::COLLECTION).await?;
    let journal_entries = fetch_all::<JournalEntry>(db, JournalEntry::COLLECTION).await?;
    let assets = fetch_all::<Asset>(db, Asset::COLLECTION).await?;
    let prompts = fetch_all::<Prompt>(db, Prompt::COLLECTION).await?;
    let prompt_versions = fetch_all::<PromptVersion>(db, PromptVersion::COLLECTION).await?;
    let settings = db
        .collection::<Settings>(Settings::COLLECTION)
        .find_one(bson::doc! {})
        .await
        .context("fetch singleton settings")?;

    // Catalog ids (categories, pitches) and prompt names stay as-is in
    // Postgres, so they don't need IdMap entries — the write pass references
    // them directly.
    let mut map = IdMap::new();
    mint_for(&mut map, Kind::Company, companies.iter().map(|c| &c.id));
    mint_for(&mut map, Kind::Question, questions.iter().map(|q| &q.id));
    mint_for(&mut map, Kind::Session, sessions.iter().map(|s| &s.id));
    mint_for(
        &mut map,
        Kind::Evaluation,
        evaluations.iter().map(|e| &e.id),
    );
    mint_for(&mut map, Kind::Summary, summaries.iter().map(|s| &s.id));
    mint_for(&mut map, Kind::Story, stories.iter().map(|s| &s.id));
    mint_for(
        &mut map,
        Kind::StoryVersion,
        story_versions.iter().map(|v| &v.id),
    );
    mint_for(
        &mut map,
        Kind::PitchVersion,
        pitch_versions.iter().map(|v| &v.id),
    );
    mint_for(
        &mut map,
        Kind::JournalEntry,
        journal_entries.iter().map(|j| &j.id),
    );
    mint_for(&mut map, Kind::Asset, assets.iter().map(|a| &a.id));
    mint_for(
        &mut map,
        Kind::PromptVersion,
        prompt_versions.iter().map(|v| &v.id),
    );

    log_summary(
        &categories,
        &companies,
        &questions,
        &sessions,
        &evaluations,
        &summaries,
        &stories,
        &story_versions,
        &pitches,
        &pitch_versions,
        &journal_entries,
        &assets,
        &prompts,
        &prompt_versions,
        settings.is_some(),
    );

    Ok(ReadSet {
        categories,
        companies,
        questions,
        sessions,
        evaluations,
        summaries,
        stories,
        story_versions,
        pitches,
        pitch_versions,
        journal_entries,
        assets,
        prompts,
        prompt_versions,
        settings,
        map,
    })
}

async fn fetch_all<T>(db: &Database, name: &'static str) -> Result<Vec<T>>
where
    T: serde::de::DeserializeOwned + Send + Sync + Unpin + 'static,
{
    let col: Collection<T> = db.collection(name);
    let cursor = col
        .find(bson::doc! {})
        .await
        .with_context(|| format!("find on collection {name}"))?;
    let docs: Vec<T> = cursor
        .try_collect()
        .await
        .with_context(|| format!("collect from {name}"))?;
    Ok(docs)
}

fn mint_for<'a, I>(map: &mut IdMap, kind: Kind, ids: I)
where
    I: IntoIterator<Item = &'a String>,
{
    for old in ids {
        let _ = map.mint(kind, old);
    }
}

#[allow(clippy::too_many_arguments)]
fn log_summary(
    categories: &[Category],
    companies: &[Company],
    questions: &[Question],
    sessions: &[Session],
    evaluations: &[Evaluation],
    summaries: &[Summary],
    stories: &[Story],
    story_versions: &[StoryVersion],
    pitches: &[Pitch],
    pitch_versions: &[PitchVersion],
    journal_entries: &[JournalEntry],
    assets: &[Asset],
    prompts: &[Prompt],
    prompt_versions: &[PromptVersion],
    has_settings: bool,
) {
    tracing::info!(
        event = "import.read.done",
        categories = categories.len(),
        companies = companies.len(),
        questions = questions.len(),
        sessions = sessions.len(),
        evaluations = evaluations.len(),
        summaries = summaries.len(),
        stories = stories.len(),
        story_versions = story_versions.len(),
        pitches = pitches.len(),
        pitch_versions = pitch_versions.len(),
        journal_entries = journal_entries.len(),
        assets = assets.len(),
        prompts = prompts.len(),
        prompt_versions = prompt_versions.len(),
        settings = has_settings,
        "read pass complete"
    );
}

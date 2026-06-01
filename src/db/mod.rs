use mongodb::error::{ErrorKind, WriteFailure};
use mongodb::{Client, Collection, Database};

use crate::models::{Category, PitchVersion, PromptVersion, Story, StoryVersion};

/// MongoDB error code for a duplicate-key violation against a unique index.
const DUPLICATE_KEY_CODE: i32 = 11000;

/// `true` when `err` is a duplicate-key violation. Callers that wrap a write
/// in a "next monotonic id" lookup use this to retry once on the natural
/// race between two concurrent inserts.
pub fn is_duplicate_key(err: &mongodb::error::Error) -> bool {
    match err.kind.as_ref() {
        ErrorKind::Write(WriteFailure::WriteError(we)) => we.code == DUPLICATE_KEY_CODE,
        _ => false,
    }
}

// Note: bypass accessors (`pub fn companies`, `pub fn assets`) were removed in
// Stage 2 of the hygiene plan. Use `crate::services::company_store` and
// `crate::services::asset_store` instead — they own these collections and
// keep route handlers off raw `db.collection::<…>` calls.

pub async fn connect(uri: &str, db_name: &str) -> anyhow::Result<Database> {
    let client = Client::with_uri_str(uri).await?;
    let db = client.database(db_name);
    db.run_command(bson::doc! { "ping": 1 }).await?;
    tracing::info!(db = db_name, "MongoDB connected");
    Ok(db)
}

pub async fn ensure_indexes(db: &Database) -> anyhow::Result<()> {
    use mongodb::options::IndexOptions;
    use mongodb::IndexModel;

    // Asset indexes moved to Postgres in Phase A Step 5 — see
    // `assets_one_primary_per_owner_kind_uidx` in migration 0003.

    let versions: Collection<PromptVersion> = db.collection(PromptVersion::COLLECTION);
    let pname_idx = IndexModel::builder()
        .keys(bson::doc! { "prompt_name": 1, "created_at": -1 })
        .options(
            IndexOptions::builder()
                .name("prompt_name_created_at".to_string())
                .build(),
        )
        .build();
    versions.create_index(pname_idx).await?;

    // Companies + questions indexes moved to Postgres in Phase A Step 6 —
    // see migration 0001 (`questions_company_id_idx`) and 0003
    // (`companies_owner_id_idx`).

    // Sessions, evaluations, summaries indexes moved to Postgres in
    // Phase A Step 7 — see migration 0001
    // (`sessions_status_started_at_idx`, `evaluations_session_id_idx`,
    // `summaries_session_id_idx`).

    let categories: Collection<Category> = db.collection(Category::COLLECTION);
    let cat_idx = IndexModel::builder()
        .keys(bson::doc! { "name": 1 })
        .options(
            IndexOptions::builder()
                .unique(true)
                .name("name_unique".to_string())
                .build(),
        )
        .build();
    categories.create_index(cat_idx).await?;

    let stories: Collection<Story> = db.collection(Story::COLLECTION);
    let story_idx = IndexModel::builder()
        .keys(bson::doc! { "competency_id": 1, "status": 1, "updated_at": -1 })
        .options(
            IndexOptions::builder()
                .name("competency_status_updated".to_string())
                .build(),
        )
        .build();
    stories.create_index(story_idx).await?;

    // Journal entry indexes moved to Postgres in Phase A Step 5 — see
    // `journal_entries_owner_active_updated_at_idx` in migration 0003.

    let story_versions: Collection<StoryVersion> = db.collection(StoryVersion::COLLECTION);
    // Unique on (story_id, version_n) — guards against the double-submit
    // race that two concurrent `next_version_n` reads would otherwise
    // resolve to the same monotonic value. The version stores catch the
    // duplicate-key error and retry once.
    let sv_idx = IndexModel::builder()
        .keys(bson::doc! { "story_id": 1, "version_n": -1 })
        .options(
            IndexOptions::builder()
                .unique(true)
                .name("story_version_unique".to_string())
                .build(),
        )
        .build();
    story_versions.create_index(sv_idx).await?;

    let pitch_versions: Collection<PitchVersion> = db.collection(PitchVersion::COLLECTION);
    let pv_idx = IndexModel::builder()
        .keys(bson::doc! { "pitch_id": 1, "version_n": -1 })
        .options(
            IndexOptions::builder()
                .unique(true)
                .name("pitch_version_unique".to_string())
                .build(),
        )
        .build();
    pitch_versions.create_index(pv_idx).await?;

    Ok(())
}

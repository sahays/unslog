use mongodb::error::{ErrorKind, WriteFailure};
use mongodb::{Client, Collection, Database};

use crate::models::{Category, PitchVersion, PromptVersion};

/// MongoDB error code for a duplicate-key violation against a unique index.
const DUPLICATE_KEY_CODE: i32 = 11000;

/// Postgres SQLSTATE for `unique_violation`. Surfaces on inserts that
/// collide with a UNIQUE / PRIMARY KEY index.
const PG_UNIQUE_VIOLATION: &str = "23505";

/// `true` when `err` is a duplicate-key violation. Callers that wrap a write
/// in a "next monotonic id" lookup use this to retry once on the natural
/// race between two concurrent inserts.
pub fn is_duplicate_key(err: &mongodb::error::Error) -> bool {
    match err.kind.as_ref() {
        ErrorKind::Write(WriteFailure::WriteError(we)) => we.code == DUPLICATE_KEY_CODE,
        _ => false,
    }
}

/// Postgres flavor of [`is_duplicate_key`]. Returns `true` when `err` is a
/// `23505 unique_violation`. Used by stores that wrap a "compute monotonic
/// version_n + insert" in a retry-once guard.
pub fn is_pg_duplicate_key(err: &sqlx::Error) -> bool {
    matches!(err.as_database_error().and_then(|e| e.code()), Some(c) if c == PG_UNIQUE_VIOLATION)
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

    // Stories + story_versions indexes moved to Postgres in Phase A Step 8 —
    // see `stories_competency_id_idx` / `story_versions_story_id_idx` in
    // migration 0001 and the UNIQUE(story_id, version_n) constraint there.

    // Journal entry indexes moved to Postgres in Phase A Step 5 — see
    // `journal_entries_owner_active_updated_at_idx` in migration 0003.

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

use mongodb::{Client, Collection, Database};

use crate::models::{Asset, Company, Evaluation, PromptVersion, QuestionBank, Session, Summary};

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

    let assets: Collection<Asset> = db.collection(Asset::COLLECTION);
    let primary_idx = IndexModel::builder()
        .keys(bson::doc! { "primary": 1 })
        .options(
            IndexOptions::builder()
                .name("primary_1".to_string())
                .build(),
        )
        .build();
    assets.create_index(primary_idx).await?;

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

    let companies: Collection<Company> = db.collection(Company::COLLECTION);
    let name_idx = IndexModel::builder()
        .keys(bson::doc! { "name": 1 })
        .options(
            IndexOptions::builder()
                .unique(true)
                .name("name_unique".to_string())
                .build(),
        )
        .build();
    companies.create_index(name_idx).await?;

    let banks: Collection<QuestionBank> = db.collection(QuestionBank::COLLECTION);
    let bank_idx = IndexModel::builder()
        .keys(bson::doc! { "company_id": 1 })
        .options(
            IndexOptions::builder()
                .unique(true)
                .name("company_id_unique".to_string())
                .build(),
        )
        .build();
    banks.create_index(bank_idx).await?;

    let sessions: Collection<Session> = db.collection(Session::COLLECTION);
    let s_idx = IndexModel::builder()
        .keys(bson::doc! { "company_id": 1, "started_at": -1 })
        .options(
            IndexOptions::builder()
                .name("company_started".to_string())
                .build(),
        )
        .build();
    sessions.create_index(s_idx).await?;

    let summaries: Collection<Summary> = db.collection(Summary::COLLECTION);
    let sum_idx = IndexModel::builder()
        .keys(bson::doc! { "session_id": 1 })
        .options(
            IndexOptions::builder()
                .unique(true)
                .name("session_id_unique".to_string())
                .build(),
        )
        .build();
    summaries.create_index(sum_idx).await?;
    let sum_idx2 = IndexModel::builder()
        .keys(bson::doc! { "company_id": 1, "created_at": -1 })
        .options(
            IndexOptions::builder()
                .name("company_created".to_string())
                .build(),
        )
        .build();
    summaries.create_index(sum_idx2).await?;

    let evals: Collection<Evaluation> = db.collection(Evaluation::COLLECTION);
    let e_idx = IndexModel::builder()
        .keys(bson::doc! { "session_id": 1 })
        .options(
            IndexOptions::builder()
                .name("session_id".to_string())
                .build(),
        )
        .build();
    evals.create_index(e_idx).await?;
    let e_idx2 = IndexModel::builder()
        .keys(bson::doc! { "session_id": 1, "question_id": 1 })
        .options(
            IndexOptions::builder()
                .unique(true)
                .name("session_question_unique".to_string())
                .build(),
        )
        .build();
    evals.create_index(e_idx2).await?;

    Ok(())
}

pub fn companies(db: &Database) -> Collection<Company> {
    db.collection(Company::COLLECTION)
}

pub fn assets(db: &Database) -> Collection<Asset> {
    db.collection(Asset::COLLECTION)
}

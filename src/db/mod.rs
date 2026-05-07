use mongodb::{Client, Collection, Database};

use crate::models::{Asset, PromptVersion};

pub async fn connect(uri: &str, db_name: &str) -> anyhow::Result<Database> {
    let client = Client::with_uri_str(uri).await?;
    let db = client.database(db_name);
    db.run_command(bson::doc! { "ping": 1 }).await?;
    tracing::info!(db = db_name, "MongoDB connected");
    Ok(db)
}

pub async fn ensure_indexes(db: &Database) -> anyhow::Result<()> {
    use mongodb::IndexModel;
    use mongodb::options::IndexOptions;

    let assets: Collection<Asset> = db.collection(Asset::COLLECTION);
    let primary_idx = IndexModel::builder()
        .keys(bson::doc! { "primary": 1 })
        .options(IndexOptions::builder().name("primary_1".to_string()).build())
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

    Ok(())
}

pub fn assets(db: &Database) -> Collection<Asset> {
    db.collection(Asset::COLLECTION)
}

use mongodb::{Client, Database};

pub async fn connect(uri: &str, db_name: &str) -> anyhow::Result<Database> {
    let client = Client::with_uri_str(uri).await?;
    let db = client.database(db_name);

    // Quick ping so we fail fast on a bad URI / unreachable server.
    db.run_command(bson::doc! { "ping": 1 }).await?;

    tracing::info!(db = db_name, "MongoDB connected");
    Ok(db)
}

pub async fn ensure_indexes(_db: &Database) -> anyhow::Result<()> {
    // Index creation is added phase-by-phase as collections come online.
    Ok(())
}

//! One-shot Mongo → Postgres importer (Phase A, Step 3 of the migration).
//!
//! Read every Mongo collection, mint new prefixed ids, rewrite every FK
//! through the [`id_map::IdMap`], then insert into Postgres in dependency
//! order. The bin is throwaway — deleted in a later sub-phase.
//!
//! Master user `usrmaster` is seeded before migrations 0003+ run (those
//! migrations refuse to apply without a master row). All pre-multi-user-era
//! data is backfilled to `usrmaster`.

pub mod id_map;
pub mod insert;
pub mod insert_extras;
pub mod read;

use anyhow::{Context, Result};
use mongodb::Database;
use sqlx::PgPool;

use id_map::MASTER_USER_ID;

/// Top-level options the bin passes in from CLI flags.
pub struct ImportOptions {
    /// Skip the pre-import "target already has data" guard. Used to re-run
    /// against a partially populated target.
    pub force: bool,
}

/// End-to-end orchestrator. Connects + migrates + seeds master + guards +
/// copies data. Logged counts at the end let the operator eyeball parity
/// against the Mongo side without dumping PII to the terminal.
pub async fn run(mongo: &Database, pg: &PgPool, opts: ImportOptions) -> Result<read::ReadSet> {
    bootstrap::ensure_master_and_migrate(pg)
        .await
        .context("bootstrap master user + migrations")?;
    if !opts.force {
        guard::reject_if_populated(pg)
            .await
            .context("pre-import emptiness guard")?;
    }
    let read_set = read::read_all(mongo)
        .await
        .context("read pass: fetch + mint")?;
    insert::insert_all(pg, &read_set, MASTER_USER_ID)
        .await
        .context("write pass: insert into postgres")?;
    Ok(read_set)
}

/// Migrations bootstrap. Migration 0003 raises if master is missing, so we
/// step through migrations manually and seed master between v2 and v3.
mod bootstrap {
    use anyhow::Result;
    use sqlx::migrate::Migrate;
    use sqlx::PgPool;
    use std::collections::HashSet;

    /// sqlx migration version that creates the `users` table — master must
    /// be seeded after this and before the next migration runs.
    const USERS_TABLE_VERSION: i64 = 2;

    pub async fn ensure_master_and_migrate(pool: &PgPool) -> Result<()> {
        let migrator = sqlx::migrate!("./migrations");
        let mut conn = pool.acquire().await?;
        conn.ensure_migrations_table().await?;
        let applied: HashSet<i64> = conn
            .list_applied_migrations()
            .await?
            .into_iter()
            .map(|m| m.version)
            .collect();
        if applied.contains(&USERS_TABLE_VERSION) {
            seed_master(pool).await?;
        }
        for migration in migrator.iter() {
            if migration.migration_type.is_down_migration() {
                continue;
            }
            if applied.contains(&migration.version) {
                continue;
            }
            conn.apply(migration).await?;
            if migration.version == USERS_TABLE_VERSION {
                seed_master(pool).await?;
            }
        }
        tracing::info!(
            event = "import.bootstrap.done",
            "schema present, master user seeded"
        );
        Ok(())
    }

    /// Idempotent master-user seed. Hash is a placeholder; the auth feature
    /// will rotate it via the standard reset flow when it lands.
    async fn seed_master(pool: &PgPool) -> Result<()> {
        let placeholder_hash =
            "$argon2id$v=19$m=19456,t=2,p=1$placeholder$placeholder-import-bootstrap";
        sqlx::query(
            r#"
            INSERT INTO users (id, code_hash, code_hint, label, tier, is_master)
            VALUES ($1, $2, '', 'master', 'master', TRUE)
            ON CONFLICT (id) DO NOTHING
            "#,
        )
        .bind(super::MASTER_USER_ID)
        .bind(placeholder_hash)
        .execute(pool)
        .await?;
        Ok(())
    }
}

/// Refuse to run against a populated target so accidental re-imports can't
/// silently double-write. `--force` bypasses this for retry workflows.
mod guard {
    use anyhow::{bail, Result};
    use sqlx::PgPool;

    /// Tables we count to decide whether the target is "empty". Catalog
    /// tables (categories, pitches, prompts) and the singleton settings row
    /// are excluded — they're seeded by the app at boot.
    const USER_OWNED_TABLES: &[&str] = &[
        "companies",
        "questions",
        "sessions",
        "evaluations",
        "summaries",
        "stories",
        "story_versions",
        "pitch_versions",
        "pitch_user_state",
        "journal_entries",
        "assets",
    ];

    pub async fn reject_if_populated(pool: &PgPool) -> Result<()> {
        let extra_users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id <> $1")
            .bind(super::MASTER_USER_ID)
            .fetch_one(pool)
            .await?;
        if extra_users > 0 {
            bail!(
                "users table has {} non-master rows — pass --force to import anyway",
                extra_users
            );
        }
        for table in USER_OWNED_TABLES {
            let count = count_table(pool, table).await?;
            if count > 0 {
                bail!(
                    "table {} already has {} rows — pass --force to import anyway",
                    table,
                    count
                );
            }
        }
        tracing::info!(event = "import.guard.passed", "target tables are empty");
        Ok(())
    }

    async fn count_table(pool: &PgPool, table: &str) -> Result<i64> {
        // Table names are from the hard-coded const above, never user input,
        // so the inline format is safe — sqlx doesn't bind identifiers.
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let n: i64 = sqlx::query_scalar(&sql).fetch_one(pool).await?;
        Ok(n)
    }
}

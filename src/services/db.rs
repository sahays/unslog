//! Postgres connection setup. Owns the `PgPool` build + sqlx migration
//! runner so `startup.rs` reads like pseudocode and the redaction helper
//! lives next to the only place that needs it.

use crate::error::AppError;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// Default pool ceiling. 20 is comfortably above the request concurrency
/// a single-user local app drives and well under typical Postgres
/// `max_connections=100` defaults — leaves headroom for psql + ad-hoc tools.
const MAX_CONNECTIONS: u32 = 20;

/// Postgres SQLSTATE for `unique_violation`. Surfaces on inserts that
/// collide with a UNIQUE / PRIMARY KEY index.
const PG_UNIQUE_VIOLATION: &str = "23505";

/// `true` when `err` is a `23505 unique_violation`. Used by stores that
/// wrap a "compute monotonic version_n + insert" in a retry-once guard.
pub fn is_pg_duplicate_key(err: &sqlx::Error) -> bool {
    matches!(err.as_database_error().and_then(|e| e.code()), Some(c) if c == PG_UNIQUE_VIOLATION)
}

/// Build a `PgPool` against `database_url` and run any pending sqlx
/// migrations under `./migrations`. Returns a clear `AppError` on
/// connect/migrate failure so the operator sees the "start Postgres"
/// hint at boot rather than a panic.
pub async fn connect_postgres(database_url: &str) -> Result<PgPool, AppError> {
    let pool = PgPoolOptions::new()
        .max_connections(MAX_CONNECTIONS)
        .connect(database_url)
        .await
        .map_err(|e| connect_failed(database_url, &e))?;
    tracing::info!(
        event = "db.postgres.connected",
        url_redacted = %url_without_password(database_url),
        "postgres pool ready"
    );
    run_migrations(&pool).await?;
    Ok(pool)
}

async fn run_migrations(pool: &PgPool) -> Result<(), AppError> {
    // `Migrator::iter()` enumerates everything embedded by `migrate!()`.
    // sqlx records each applied row in `_sqlx_migrations`, so this `count`
    // is the total known set — not "applied this run" — which is the right
    // observability signal here: it answers "are the schemas the binary
    // ships with all present?" without an extra query.
    let migrator = sqlx::migrate!("./migrations");
    let count = migrator.iter().count();
    migrator
        .run(pool)
        .await
        .map_err(|e| AppError::Other(anyhow::anyhow!("sqlx migrate: {e}")))?;
    tracing::info!(
        event = "db.migrations.applied",
        count = count,
        "sqlx migrations up to date"
    );
    Ok(())
}

/// Strip the password segment from a `postgres://user:pass@host/db` URL
/// for safe logging. Returns the input verbatim for malformed URLs or
/// URLs that have no `user:password@` userinfo segment.
pub fn url_without_password(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    let Some((authority_end, _)) = rest.match_indices('/').next() else {
        return format!("{scheme}://{}", redact_authority(rest));
    };
    let (authority, tail) = rest.split_at(authority_end);
    format!("{scheme}://{}{tail}", redact_authority(authority))
}

fn redact_authority(authority: &str) -> String {
    let Some((userinfo, host)) = authority.rsplit_once('@') else {
        return authority.to_string();
    };
    match userinfo.split_once(':') {
        Some((user, _pass)) => format!("{user}:***@{host}"),
        None => format!("{userinfo}@{host}"),
    }
}

fn connect_failed(database_url: &str, err: &sqlx::Error) -> AppError {
    AppError::Other(anyhow::anyhow!(
        "postgres connect to {} failed ({}). Start Postgres via ./scripts/dev-up.sh.",
        url_without_password(database_url),
        err
    ))
}

#[cfg(test)]
#[path = "db_tests.rs"]
mod tests;

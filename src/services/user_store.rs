//! CRUD over the Postgres `users` table.
//!
//! Reads always include `WHERE revoked_at IS NULL` so revoked users behave
//! like they don't exist for the rest of the app. The single exception is
//! `find_by_id_including_revoked` (used by the admin "show details" view in
//! a future step) — not exposed in this commit.
//!
//! Lookups for login do an **iterate-and-verify** dance in
//! `services::auth` (Phase 1.2): scan active users, run
//! `code_hash::verify` against each. `find_by_code_hash` is provided here
//! for the path where we already have a known hash (e.g. master-seed
//! collision check).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{User, UserTier};

/// Persistence seam. Production: [`PgUserSource`]; tests: `MockUserSource`.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait UserSource: Send + Sync {
    async fn find_by_id(&self, id: &str) -> Result<Option<User>, AppError>;
    /// Exact-hash lookup. Returns `None` when the input doesn't match any
    /// active user — does *not* run argon2 verify.
    async fn find_by_code_hash(&self, code_hash: &str) -> Result<Option<User>, AppError>;
    async fn list_non_master(&self) -> Result<Vec<User>, AppError>;
    /// Active non-master users only (revoked_at IS NULL). Used by the
    /// plain-code login scan in Phase 1.2.
    async fn list_active_non_master(&self) -> Result<Vec<User>, AppError>;
    /// Every currently-active user, master included. Used by the `/login`
    /// scan-and-verify dance — master must be able to sign in via the same
    /// path, otherwise the bootstrap account is unreachable. O(N) over a
    /// preview-scale user list; the cost is dominated by argon2 verify, not
    /// the DB fetch.
    async fn list_active_for_login(&self) -> Result<Vec<User>, AppError>;
    async fn insert(&self, user: &User) -> Result<(), AppError>;
    async fn set_expires_at(&self, id: &str, expires_at: DateTime<Utc>) -> Result<(), AppError>;
    async fn set_revoked(&self, id: &str) -> Result<(), AppError>;
    /// Stamp `activated_at` on first successful login (no-op if already set).
    async fn set_activated_at(&self, id: &str, now: DateTime<Utc>) -> Result<(), AppError>;
    async fn touch_last_seen(&self, id: &str) -> Result<(), AppError>;
}

/// Production `UserSource` over a `&PgPool`. Used by service-internal
/// callers that already hold a pool borrow.
pub struct PgUserSource<'a> {
    pub pool: &'a PgPool,
}

/// Owned counterpart for `AppState`: keeps the `PgPool` (itself `Arc`-backed
/// internally in sqlx) so the trait object can outlive any single request
/// scope. All methods just delegate to the borrowed impl, so the SQL only
/// lives in one place.
pub struct PgUserSourceOwned {
    pub pool: PgPool,
}

#[async_trait]
impl UserSource for PgUserSourceOwned {
    async fn find_by_id(&self, id: &str) -> Result<Option<User>, AppError> {
        PgUserSource { pool: &self.pool }.find_by_id(id).await
    }
    async fn find_by_code_hash(&self, code_hash: &str) -> Result<Option<User>, AppError> {
        PgUserSource { pool: &self.pool }
            .find_by_code_hash(code_hash)
            .await
    }
    async fn list_non_master(&self) -> Result<Vec<User>, AppError> {
        PgUserSource { pool: &self.pool }.list_non_master().await
    }
    async fn list_active_non_master(&self) -> Result<Vec<User>, AppError> {
        PgUserSource { pool: &self.pool }
            .list_active_non_master()
            .await
    }
    async fn list_active_for_login(&self) -> Result<Vec<User>, AppError> {
        PgUserSource { pool: &self.pool }
            .list_active_for_login()
            .await
    }
    async fn insert(&self, user: &User) -> Result<(), AppError> {
        PgUserSource { pool: &self.pool }.insert(user).await
    }
    async fn set_expires_at(&self, id: &str, expires_at: DateTime<Utc>) -> Result<(), AppError> {
        PgUserSource { pool: &self.pool }
            .set_expires_at(id, expires_at)
            .await
    }
    async fn set_revoked(&self, id: &str) -> Result<(), AppError> {
        PgUserSource { pool: &self.pool }.set_revoked(id).await
    }
    async fn set_activated_at(&self, id: &str, now: DateTime<Utc>) -> Result<(), AppError> {
        PgUserSource { pool: &self.pool }
            .set_activated_at(id, now)
            .await
    }
    async fn touch_last_seen(&self, id: &str) -> Result<(), AppError> {
        PgUserSource { pool: &self.pool }.touch_last_seen(id).await
    }
}

#[async_trait]
impl<'a> UserSource for PgUserSource<'a> {
    async fn find_by_id(&self, id: &str) -> Result<Option<User>, AppError> {
        let row: Option<UserRow> = sqlx::query_as::<_, UserRow>(SELECT_ACTIVE_WHERE_ID)
            .bind(id)
            .fetch_optional(self.pool)
            .await?;
        row.map(UserRow::try_into_user).transpose()
    }

    async fn find_by_code_hash(&self, code_hash: &str) -> Result<Option<User>, AppError> {
        let row: Option<UserRow> = sqlx::query_as::<_, UserRow>(SELECT_ACTIVE_WHERE_HASH)
            .bind(code_hash)
            .fetch_optional(self.pool)
            .await?;
        row.map(UserRow::try_into_user).transpose()
    }

    async fn list_non_master(&self) -> Result<Vec<User>, AppError> {
        list_where(self.pool, SELECT_ALL_NON_MASTER).await
    }

    async fn list_active_non_master(&self) -> Result<Vec<User>, AppError> {
        list_where(self.pool, SELECT_ACTIVE_NON_MASTER).await
    }

    async fn list_active_for_login(&self) -> Result<Vec<User>, AppError> {
        list_where(self.pool, SELECT_ACTIVE_FOR_LOGIN).await
    }

    async fn insert(&self, user: &User) -> Result<(), AppError> {
        validate_invariants(user)?;
        sqlx::query(INSERT_USER_SQL)
            .bind(&user.id)
            .bind(&user.code_hash)
            .bind(&user.code_hint)
            .bind(&user.label)
            .bind(user.tier.as_str())
            .bind(user.is_master)
            .bind(user.activated_at)
            .bind(user.expires_at)
            .bind(user.last_seen_at)
            .bind(user.revoked_at)
            .bind(&user.invited_by)
            .bind(user.created_at)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    async fn set_expires_at(&self, id: &str, expires_at: DateTime<Utc>) -> Result<(), AppError> {
        sqlx::query("UPDATE users SET expires_at = $2 WHERE id = $1 AND revoked_at IS NULL")
            .bind(id)
            .bind(expires_at)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    async fn set_revoked(&self, id: &str) -> Result<(), AppError> {
        sqlx::query("UPDATE users SET revoked_at = NOW() WHERE id = $1 AND revoked_at IS NULL")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    async fn touch_last_seen(&self, id: &str) -> Result<(), AppError> {
        sqlx::query("UPDATE users SET last_seen_at = NOW() WHERE id = $1 AND revoked_at IS NULL")
            .bind(id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    async fn set_activated_at(&self, id: &str, now: DateTime<Utc>) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE users SET activated_at = $2 \
             WHERE id = $1 AND revoked_at IS NULL AND activated_at IS NULL",
        )
        .bind(id)
        .bind(now)
        .execute(self.pool)
        .await?;
        Ok(())
    }
}

async fn list_where(pool: &PgPool, sql: &str) -> Result<Vec<User>, AppError> {
    let rows: Vec<UserRow> = sqlx::query_as::<_, UserRow>(sql).fetch_all(pool).await?;
    rows.into_iter().map(UserRow::try_into_user).collect()
}

/// Insert-time invariants that mirror the `users_master_consistency_chk`
/// CHECK in 0002, plus tier-vs-is_master sanity. Surfaces a clear
/// `BadRequest` instead of a Postgres-side CHECK error.
fn validate_invariants(user: &User) -> Result<(), AppError> {
    let bad = |msg: &str| Err(AppError::BadRequest(msg.to_string()));
    if user.is_master {
        if user.tier != UserTier::Master {
            return bad("master user must have tier=master");
        }
        if user.expires_at.is_some() {
            return bad("master user must not have expires_at");
        }
        return Ok(());
    }
    if matches!(user.tier, UserTier::Master) {
        return bad("non-master user must not have tier=master");
    }
    Ok(())
}

// ── Pool-facing convenience wrappers ────────────────────────────────────

pub async fn find_by_id(pool: &PgPool, id: &str) -> Result<Option<User>, AppError> {
    PgUserSource { pool }.find_by_id(id).await
}

pub async fn list_non_master(pool: &PgPool) -> Result<Vec<User>, AppError> {
    PgUserSource { pool }.list_non_master().await
}

pub async fn list_active_non_master(pool: &PgPool) -> Result<Vec<User>, AppError> {
    PgUserSource { pool }.list_active_non_master().await
}

pub async fn list_active_for_login(pool: &PgPool) -> Result<Vec<User>, AppError> {
    PgUserSource { pool }.list_active_for_login().await
}

pub async fn set_activated_at(pool: &PgPool, id: &str, now: DateTime<Utc>) -> Result<(), AppError> {
    PgUserSource { pool }.set_activated_at(id, now).await
}

pub async fn insert(pool: &PgPool, user: &User) -> Result<(), AppError> {
    PgUserSource { pool }.insert(user).await
}

pub async fn touch_last_seen(pool: &PgPool, id: &str) -> Result<(), AppError> {
    PgUserSource { pool }.touch_last_seen(id).await
}

pub async fn set_revoked(pool: &PgPool, id: &str) -> Result<(), AppError> {
    PgUserSource { pool }.set_revoked(id).await
}

pub async fn set_expires_at(
    pool: &PgPool,
    id: &str,
    expires_at: DateTime<Utc>,
) -> Result<(), AppError> {
    PgUserSource { pool }.set_expires_at(id, expires_at).await
}

// ── SQL + row mapper (TEXT enum → typed `UserTier`) ─────────────────────

const USER_COLS: &str = "id, code_hash, code_hint, label, tier, is_master,
    activated_at, expires_at, last_seen_at, revoked_at, invited_by, created_at";

const SELECT_ACTIVE_WHERE_ID: &str = "SELECT id, code_hash, code_hint, label, tier, is_master, \
     activated_at, expires_at, last_seen_at, revoked_at, invited_by, created_at \
     FROM users WHERE id = $1 AND revoked_at IS NULL";

const SELECT_ACTIVE_WHERE_HASH: &str = "SELECT id, code_hash, code_hint, label, tier, is_master, \
     activated_at, expires_at, last_seen_at, revoked_at, invited_by, created_at \
     FROM users WHERE code_hash = $1 AND revoked_at IS NULL";

const SELECT_ALL_NON_MASTER: &str = "SELECT id, code_hash, code_hint, label, tier, is_master, \
     activated_at, expires_at, last_seen_at, revoked_at, invited_by, created_at \
     FROM users WHERE is_master = FALSE ORDER BY created_at DESC";

const SELECT_ACTIVE_NON_MASTER: &str = "SELECT id, code_hash, code_hint, label, tier, is_master, \
     activated_at, expires_at, last_seen_at, revoked_at, invited_by, created_at \
     FROM users WHERE is_master = FALSE AND revoked_at IS NULL \
     ORDER BY created_at DESC";

const SELECT_ACTIVE_FOR_LOGIN: &str = "SELECT id, code_hash, code_hint, label, tier, is_master, \
     activated_at, expires_at, last_seen_at, revoked_at, invited_by, created_at \
     FROM users WHERE revoked_at IS NULL \
     ORDER BY is_master DESC, created_at DESC";

const INSERT_USER_SQL: &str =
    "INSERT INTO users (id, code_hash, code_hint, label, tier, is_master, \
     activated_at, expires_at, last_seen_at, revoked_at, invited_by, created_at) \
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)";

#[derive(sqlx::FromRow)]
struct UserRow {
    id: String,
    code_hash: String,
    code_hint: String,
    label: String,
    tier: String,
    is_master: bool,
    activated_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
    last_seen_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    invited_by: Option<String>,
    created_at: DateTime<Utc>,
}

impl UserRow {
    fn try_into_user(self) -> Result<User, AppError> {
        let tier = UserTier::parse(&self.tier).ok_or_else(|| {
            AppError::Other(anyhow::anyhow!(
                "users.tier {:?} not in CHECK set",
                self.tier
            ))
        })?;
        Ok(User {
            id: self.id,
            code_hash: self.code_hash,
            code_hint: self.code_hint,
            label: self.label,
            tier,
            is_master: self.is_master,
            activated_at: self.activated_at,
            expires_at: self.expires_at,
            last_seen_at: self.last_seen_at,
            revoked_at: self.revoked_at,
            invited_by: self.invited_by,
            created_at: self.created_at,
        })
    }
}

// `USER_COLS` lives so future code paths can build dynamic SELECTs from
// one source of truth. Silence dead_code while only the const-strings
// above use it directly.
#[allow(dead_code)]
const _USER_COLS_SENTINEL: &str = USER_COLS;

#[cfg(test)]
#[path = "user_store_tests.rs"]
mod tests;

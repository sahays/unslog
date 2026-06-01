//! Production `MeteringDeps` implementation over Postgres.
//!
//! Three columns of policy share this file:
//!   * `sum_units_window` — `SUM(cost_units)` over the user's rolling
//!     window. Uses the `(user_id, ts DESC)` btree from migration 0005.
//!   * `tier_cap_for` — env-driven Pro / Pro-Max / Master mapping; lives
//!     here instead of on `AppConfig` so the policy is hot-swappable when
//!     we move tier caps into the `settings` row.
//!   * `append` — single-row INSERT into `request_log` with the kind text
//!     pre-validated by the `RequestKind` enum.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::config::AppConfig;
use crate::error::AppError;
use crate::models::UserTier;
use crate::services::auth::CurrentUser;
use crate::services::metering::cap::AppendRow;
use crate::services::metering::MeteringDeps;

/// Production `MeteringDeps` over a `&PgPool`. Mirrors the borrowed +
/// owned split used by `UserSource`, `LoginThrottle`, etc.
pub struct PgMeteringDeps<'a> {
    pub pool: &'a PgPool,
    pub config: &'a AppConfig,
}

/// Owned counterpart for `AppState`.
pub struct PgMeteringDepsOwned {
    pub pool: PgPool,
    pub config: Arc<AppConfig>,
}

#[async_trait]
impl MeteringDeps for PgMeteringDepsOwned {
    async fn sum_units_window(&self, user_id: &str, since: DateTime<Utc>) -> Result<u64, AppError> {
        self.borrowed().sum_units_window(user_id, since).await
    }
    fn tier_cap_for(&self, user: &CurrentUser) -> Option<u32> {
        self.borrowed().tier_cap_for(user)
    }
    async fn append(&self, row: AppendRow) -> Result<(), AppError> {
        self.borrowed().append(row).await
    }
}

impl PgMeteringDepsOwned {
    fn borrowed(&self) -> PgMeteringDeps<'_> {
        PgMeteringDeps {
            pool: &self.pool,
            config: &self.config,
        }
    }
}

#[async_trait]
impl<'a> MeteringDeps for PgMeteringDeps<'a> {
    async fn sum_units_window(&self, user_id: &str, since: DateTime<Utc>) -> Result<u64, AppError> {
        let total: Option<i64> = sqlx::query_scalar(
            "SELECT COALESCE(SUM(cost_units), 0)::BIGINT \
             FROM request_log \
             WHERE user_id = $1 AND ts > $2",
        )
        .bind(user_id)
        .bind(since)
        .fetch_one(self.pool)
        .await?;
        Ok(total.unwrap_or(0).max(0) as u64)
    }

    fn tier_cap_for(&self, user: &CurrentUser) -> Option<u32> {
        match user.tier {
            UserTier::Master => None,
            UserTier::Pro => Some(self.config.pro_request_cap_daily),
            UserTier::ProMax => Some(self.config.pro_max_request_cap_daily),
        }
    }

    async fn append(&self, row: AppendRow) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO request_log \
             (user_id, method, path, status, duration_ms, kind, cost_units) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&row.user_id)
        .bind(&row.method)
        .bind(&row.path)
        .bind(row.status)
        .bind(row.duration_ms)
        .bind(&row.kind)
        .bind(row.cost_units as i32)
        .execute(self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "request_log_store_tests.rs"]
mod tests;

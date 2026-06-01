//! IP-scoped brute-force throttle for `/login`.
//!
//! Backed by `login_attempts` (migration 0002). Counts failures within a
//! rolling window; a successful login records as `success = TRUE` so the
//! audit trail keeps both sides of the picture but **does not** reset the
//! failure counter — a window-based decay is harder to bypass than a
//! point-reset, and avoids the "burst until win → burst again" pattern.
//!
//! Forensic field stored per attempt is the first-4 chars of the code
//! (`code_prefix`). Never the full code; never the hash.

use async_trait::async_trait;
use sqlx::PgPool;
use std::net::IpAddr;

use crate::error::AppError;

/// Persistence seam. Production: [`PgLoginThrottle`]; tests: `MockLoginThrottle`.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait LoginThrottle: Send + Sync {
    /// Returns `Ok(())` when the IP has under-cap failures in the window.
    /// Returns [`AppError::RateLimited`] when at-or-over cap.
    async fn check(&self, ip: &IpAddr, max_attempts: u32, window_secs: u64)
        -> Result<(), AppError>;

    /// Record a failed attempt. `code_prefix` MUST be at most 4 chars —
    /// callers compute it via `invite_codes::prefix`.
    async fn record_failure(&self, ip: &IpAddr, code_prefix: &str) -> Result<(), AppError>;

    /// Record a successful attempt. Does not zero out failures (see module
    /// docs); future window roll-off does that organically.
    async fn record_success(&self, ip: &IpAddr, code_prefix: &str) -> Result<(), AppError>;
}

/// Production `LoginThrottle` over a `&PgPool`.
pub struct PgLoginThrottle<'a> {
    pub pool: &'a PgPool,
}

/// Owned counterpart for `AppState`. Delegates to the borrowed impl so
/// the SQL lives in one place.
pub struct PgLoginThrottleOwned {
    pub pool: PgPool,
}

#[async_trait]
impl LoginThrottle for PgLoginThrottleOwned {
    async fn check(
        &self,
        ip: &IpAddr,
        max_attempts: u32,
        window_secs: u64,
    ) -> Result<(), AppError> {
        PgLoginThrottle { pool: &self.pool }
            .check(ip, max_attempts, window_secs)
            .await
    }
    async fn record_failure(&self, ip: &IpAddr, code_prefix: &str) -> Result<(), AppError> {
        PgLoginThrottle { pool: &self.pool }
            .record_failure(ip, code_prefix)
            .await
    }
    async fn record_success(&self, ip: &IpAddr, code_prefix: &str) -> Result<(), AppError> {
        PgLoginThrottle { pool: &self.pool }
            .record_success(ip, code_prefix)
            .await
    }
}

#[async_trait]
impl<'a> LoginThrottle for PgLoginThrottle<'a> {
    async fn check(
        &self,
        ip: &IpAddr,
        max_attempts: u32,
        window_secs: u64,
    ) -> Result<(), AppError> {
        let window_secs_f = window_secs as f64;
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM login_attempts \
             WHERE ip = $1 AND success = FALSE \
             AND attempted_at > NOW() - make_interval(secs => $2)",
        )
        .bind(*ip)
        .bind(window_secs_f)
        .fetch_one(self.pool)
        .await?;
        if (count as u64) >= max_attempts as u64 {
            return Err(AppError::RateLimited {
                used: count as u32,
                cap: max_attempts,
            });
        }
        Ok(())
    }

    async fn record_failure(&self, ip: &IpAddr, code_prefix: &str) -> Result<(), AppError> {
        insert_attempt(self.pool, ip, code_prefix, false).await
    }

    async fn record_success(&self, ip: &IpAddr, code_prefix: &str) -> Result<(), AppError> {
        insert_attempt(self.pool, ip, code_prefix, true).await
    }
}

async fn insert_attempt(
    pool: &PgPool,
    ip: &IpAddr,
    code_prefix: &str,
    success: bool,
) -> Result<(), AppError> {
    let trimmed = clip_prefix(code_prefix);
    sqlx::query("INSERT INTO login_attempts (ip, code_prefix, success) VALUES ($1, $2, $3)")
        .bind(*ip)
        .bind(&trimmed)
        .bind(success)
        .execute(pool)
        .await?;
    Ok(())
}

/// Clip to at most 4 chars to satisfy the
/// `login_attempts_prefix_len_chk` CHECK. We accept the caller's input
/// rather than panicking; defense-in-depth for the trait contract.
fn clip_prefix(s: &str) -> String {
    s.chars()
        .take(crate::services::invite_codes::PREFIX_LEN)
        .collect()
}

// ── Pool-facing convenience wrappers ────────────────────────────────────

pub async fn check(
    pool: &PgPool,
    ip: &IpAddr,
    max_attempts: u32,
    window_secs: u64,
) -> Result<(), AppError> {
    PgLoginThrottle { pool }
        .check(ip, max_attempts, window_secs)
        .await
}

pub async fn record_failure(pool: &PgPool, ip: &IpAddr, code_prefix: &str) -> Result<(), AppError> {
    PgLoginThrottle { pool }
        .record_failure(ip, code_prefix)
        .await
}

pub async fn record_success(pool: &PgPool, ip: &IpAddr, code_prefix: &str) -> Result<(), AppError> {
    PgLoginThrottle { pool }
        .record_success(ip, code_prefix)
        .await
}

#[cfg(test)]
#[path = "login_throttle_tests.rs"]
mod tests;

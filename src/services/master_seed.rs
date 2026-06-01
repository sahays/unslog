//! Idempotent master-user bootstrap, called from `startup::run` right
//! after migrations apply.
//!
//! - First boot with a real `MASTER_INVITE_CODE`: argon2-hash the code,
//!   insert `usrmaster`. Idempotent via `ON CONFLICT DO NOTHING`.
//! - Subsequent boots with the same code: no-op (lookup matches).
//! - Subsequent boots with a *different* code: log a warning, do not
//!   overwrite. Avoids the "operator typo locks themselves out" scenario.
//! - Missing or malformed code: hard error — refuses to boot.

use chrono::Utc;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{User, UserTier};
use crate::services::{code_hash, invite_codes, user_store};

/// Reserved literal id for the master user. Matches the `users.id` CHECK
/// constraint and the FK targets used by per-row owner backfills.
pub const MASTER_ID: &str = "usrmaster";

/// Ensure exactly one `usrmaster` row exists and is consistent with the
/// configured code. Safe to call on every boot.
pub async fn ensure_master(pool: &PgPool, code: &str, label: &str) -> Result<(), AppError> {
    validate_code_format(code)?;
    if let Some(existing) = user_store::find_by_id(pool, MASTER_ID).await? {
        warn_if_code_changed(&existing, code);
        return Ok(());
    }
    insert_fresh_master(pool, code, label).await
}

fn validate_code_format(code: &str) -> Result<(), AppError> {
    if !invite_codes::is_valid_format(code) {
        return Err(AppError::Other(anyhow::anyhow!(
            "MASTER_INVITE_CODE must be 12 alphanumerics (set it in .env)"
        )));
    }
    Ok(())
}

async fn insert_fresh_master(pool: &PgPool, code: &str, label: &str) -> Result<(), AppError> {
    let hash = code_hash::hash(code)?;
    let prefix = invite_codes::prefix(code);
    let user = User {
        id: MASTER_ID.to_string(),
        code_hash: hash,
        code_hint: invite_codes::hint(code),
        label: label.to_string(),
        tier: UserTier::Master,
        is_master: true,
        activated_at: None,
        expires_at: None,
        last_seen_at: None,
        revoked_at: None,
        invited_by: None,
        created_at: Utc::now(),
    };
    match user_store::insert(pool, &user).await {
        Ok(()) => {
            tracing::info!(
                event = "auth.master_seed",
                success = true,
                code_prefix = %prefix,
                "master user seeded"
            );
            Ok(())
        }
        Err(AppError::Sqlx(e)) if crate::services::db::is_pg_duplicate_key(&e) => {
            // Concurrent boot won the race; treat as already-seeded.
            tracing::info!(
                event = "auth.master_seed",
                success = true,
                code_prefix = %prefix,
                "master user already present (race)"
            );
            Ok(())
        }
        Err(e) => {
            tracing::error!(
                event = "auth.master_seed",
                success = false,
                code_prefix = %prefix,
                "master seed insert failed"
            );
            Err(e)
        }
    }
}

fn warn_if_code_changed(existing: &User, code: &str) {
    let prefix = invite_codes::prefix(code);
    if code_hash::verify(code, &existing.code_hash) {
        tracing::info!(
            event = "auth.master_seed",
            success = true,
            code_prefix = %prefix,
            "master user already seeded; code matches"
        );
    } else {
        tracing::warn!(
            event = "auth.master_seed",
            success = false,
            code_prefix = %prefix,
            "MASTER_INVITE_CODE differs from stored hash; not overwriting. Delete row or rotate via admin to change."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_validate_code_format_rejects_blank() {
        let err = validate_code_format("").expect_err("err");
        assert!(matches!(err, AppError::Other(_)));
    }

    #[test]
    fn case_validate_code_format_rejects_short() {
        assert!(validate_code_format("abc").is_err());
    }

    #[test]
    fn case_validate_code_format_rejects_bad_charset() {
        assert!(validate_code_format("aaaaa-aaaaaa").is_err());
    }

    #[test]
    fn case_validate_code_format_accepts_12_alphanum() {
        validate_code_format("GPbb5GAnsEQZ").expect("ok");
        validate_code_format("aaaaaaaaaaaa").expect("ok");
    }
}

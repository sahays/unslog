//! Idempotent master-user bootstrap, called from `startup::run` right
//! after migrations apply.
//!
//! - First boot with a real `MASTER_INVITE_CODE` on a fresh DB: migration
//!   0003 has already inserted a row with [`PLACEHOLDER_CODE_HASH`]; this
//!   function recognises the placeholder and overwrites it with the
//!   argon2id hash of the configured code.
//! - First boot on a legacy DB (pre-self-bootstrap migration): no row
//!   exists yet, so we INSERT one with the real hash.
//! - Subsequent boots with the same code: no-op (verify matches).
//! - Subsequent boots with a *different* real code: log a warning, do not
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

/// Sentinel hash inserted by migration 0003 when no master row exists.
/// Recognised by `ensure_master` and force-overwritten with the real
/// argon2id hash on first boot. The string is intentionally not a valid
/// argon2id PHC — `code_hash::verify` returns `false` against it for any
/// candidate, so a partial bootstrap can never accidentally authenticate.
pub const PLACEHOLDER_CODE_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$placeholder$placeholder";

/// Ensure exactly one `usrmaster` row exists and is consistent with the
/// configured code. Safe to call on every boot.
pub async fn ensure_master(pool: &PgPool, code: &str, label: &str) -> Result<(), AppError> {
    validate_code_format(code)?;
    match user_store::find_by_id(pool, MASTER_ID).await? {
        Some(existing) if existing.code_hash == PLACEHOLDER_CODE_HASH => {
            overwrite_placeholder(pool, code, label).await
        }
        Some(existing) => {
            warn_if_code_changed(&existing, code);
            Ok(())
        }
        None => insert_fresh_master(pool, code, label).await,
    }
}

/// Replace the migration-seeded placeholder row's hash/hint/label with the
/// real values derived from the configured invite code. Single owner-scoped
/// UPDATE so no `users_single_master_uidx` race is possible.
async fn overwrite_placeholder(pool: &PgPool, code: &str, label: &str) -> Result<(), AppError> {
    let hash = code_hash::hash(code)?;
    let hint = invite_codes::hint(code);
    let prefix = invite_codes::prefix(code);
    sqlx::query(
        "UPDATE users \
         SET code_hash = $2, code_hint = $3, label = $4 \
         WHERE id = $1 AND code_hash = $5",
    )
    .bind(MASTER_ID)
    .bind(&hash)
    .bind(&hint)
    .bind(label)
    .bind(PLACEHOLDER_CODE_HASH)
    .execute(pool)
    .await?;
    tracing::info!(
        event = "auth.master_seed",
        success = true,
        code_prefix = %prefix,
        "placeholder master hash overwritten with real argon2id hash"
    );
    Ok(())
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

    #[test]
    fn case_placeholder_hash_is_not_a_verifiable_phc() {
        // The placeholder string must never accidentally authenticate any
        // candidate — `code_hash::verify` returns `false` for it.
        assert!(!code_hash::verify("GPbb5GAnsEQZ", PLACEHOLDER_CODE_HASH));
        assert!(!code_hash::verify("placeholder", PLACEHOLDER_CODE_HASH));
        assert!(!code_hash::verify("", PLACEHOLDER_CODE_HASH));
    }

    /// `#[sqlx::test]` migration runs in a fresh DB → 0003 inserts the
    /// placeholder row. `ensure_master` must overwrite that hash so the
    /// configured invite code can authenticate from boot one.
    #[sqlx::test(migrations = "./migrations")]
    async fn case_first_boot_overwrites_placeholder_hash(pool: sqlx::PgPool) {
        let row = user_store::find_by_id(&pool, MASTER_ID)
            .await
            .expect("master row present after migrations")
            .expect("placeholder inserted by 0003");
        assert_eq!(row.code_hash, PLACEHOLDER_CODE_HASH);

        ensure_master(&pool, "GPbb5GAnsEQZ", "test-master")
            .await
            .expect("ensure_master ok");

        let post = user_store::find_by_id(&pool, MASTER_ID)
            .await
            .expect("post-seed lookup ok")
            .expect("master row still present");
        assert_ne!(post.code_hash, PLACEHOLDER_CODE_HASH);
        assert!(code_hash::verify("GPbb5GAnsEQZ", &post.code_hash));
        assert_eq!(post.label, "test-master");
    }

    /// Idempotent: calling `ensure_master` twice with the same real code
    /// leaves the row untouched on the second call.
    #[sqlx::test(migrations = "./migrations")]
    async fn case_second_boot_with_same_code_is_idempotent(pool: sqlx::PgPool) {
        ensure_master(&pool, "GPbb5GAnsEQZ", "test-master")
            .await
            .expect("first ok");
        let first = user_store::find_by_id(&pool, MASTER_ID)
            .await
            .expect("first lookup")
            .expect("present");
        ensure_master(&pool, "GPbb5GAnsEQZ", "test-master")
            .await
            .expect("second ok");
        let second = user_store::find_by_id(&pool, MASTER_ID)
            .await
            .expect("second lookup")
            .expect("present");
        // Same hash retained — no churn.
        assert_eq!(first.code_hash, second.code_hash);
    }

    /// After the placeholder is overwritten, supplying a *different* real
    /// code must NOT silently overwrite the stored hash. It logs a warning
    /// and returns Ok (safe path), leaving the existing hash in place.
    #[sqlx::test(migrations = "./migrations")]
    async fn case_different_real_code_does_not_overwrite(pool: sqlx::PgPool) {
        ensure_master(&pool, "GPbb5GAnsEQZ", "test-master")
            .await
            .expect("first ok");
        let first = user_store::find_by_id(&pool, MASTER_ID)
            .await
            .expect("ok")
            .expect("present");
        ensure_master(&pool, "XYZqwertyAB9", "ignored")
            .await
            .expect("returns ok with warning");
        let second = user_store::find_by_id(&pool, MASTER_ID)
            .await
            .expect("ok")
            .expect("present");
        assert_eq!(first.code_hash, second.code_hash);
        assert_eq!(first.label, second.label);
    }
}

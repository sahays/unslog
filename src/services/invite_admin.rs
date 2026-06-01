//! Master-only invite-code administration.
//!
//! Backs `/settings/invites` — minting new codes, listing existing
//! invitees with usage, extending expirations, and revoking access. The
//! plain invite code is generated here exactly once and returned to the
//! caller; it is **never** stored, logged, or otherwise persisted. Only
//! the argon2id hash ends up in `users.code_hash`, and only the
//! forensic `code_hint` (first 4 + "…" + last 2) reaches logs.
//!
//! Trait-shaped (`InviteAdminDeps`) so route handlers can swap a mock in
//! unit tests without standing up Postgres.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::UserTier;
use crate::services::{code_hash, invite_codes};

/// Allowed validity windows for a freshly minted (or extended) invite.
/// Keeping this a closed set means the admin UI's `<select>` doesn't have
/// to round-trip a free-form integer and the trust boundary stays narrow.
const ALLOWED_DAYS: &[u32] = &[30, 60, 90];

/// Upper bound on the human-readable label that's stored next to the row.
/// Mirrors the `<input maxlength="64">` in the form template.
const MAX_LABEL_LEN: usize = 64;

/// Retry budget on insert collisions. The id space is `usr` + 6 lowercase
/// alphanumerics (36^6 ≈ 2.1B) and the `code_hash` is argon2 over a 71-bit
/// code — a 3-attempt budget is comfortable above any realistic collision.
const INSERT_MAX_ATTEMPTS: u8 = 3;

/// Charset for the 6-char random suffix on a new user id. Matches the
/// `users.id ~ '^usr[a-z0-9]{6}$'` CHECK in migration 0002.
const ID_ALPHABET: &[u8; 36] = b"abcdefghijklmnopqrstuvwxyz0123456789";

/// What the caller asks for when minting an invite.
pub struct MintInput {
    pub label: String,
    pub tier: UserTier,
    pub days: u32,
}

/// Single-shot return value from a successful mint. `plain_code` is the
/// only place this value will ever appear after this call returns — it
/// is NOT stored and NOT logged.
#[derive(Debug)]
pub struct MintOutput {
    pub id: String,
    pub plain_code: String,
    pub code_hint: String,
    pub expires_at: DateTime<Utc>,
}

/// One row on the admin "active invitees" table. `used_units` is the
/// running 30-day cost-units sum from `request_log` for this user.
#[derive(Debug)]
pub struct InviteeRow {
    pub id: String,
    pub label: String,
    pub tier: UserTier,
    pub code_hint: String,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub activated_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub used_units: u64,
}

/// Persistence seam consumed by `/settings/invites` handlers.
/// Production: [`PgInviteAdminDeps`]; tests: `MockInviteAdminDeps`.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait InviteAdminDeps: Send + Sync {
    async fn list_invitees(&self) -> Result<Vec<InviteeRow>, AppError>;
    async fn mint(&self, input: MintInput) -> Result<MintOutput, AppError>;
    async fn extend(&self, invitee_id: &str, days: u32) -> Result<DateTime<Utc>, AppError>;
    async fn revoke(&self, invitee_id: &str) -> Result<(), AppError>;
}

/// Production `InviteAdminDeps` backed by a Postgres pool.
pub struct PgInviteAdminDeps {
    pub pool: PgPool,
}

#[async_trait]
impl InviteAdminDeps for PgInviteAdminDeps {
    async fn list_invitees(&self) -> Result<Vec<InviteeRow>, AppError> {
        list_invitees_sql(&self.pool).await
    }

    async fn mint(&self, input: MintInput) -> Result<MintOutput, AppError> {
        validate_mint_input(&input)?;
        mint_with_retry(&self.pool, &input).await
    }

    async fn extend(&self, invitee_id: &str, days: u32) -> Result<DateTime<Utc>, AppError> {
        validate_days(days)?;
        extend_sql(&self.pool, invitee_id, days).await
    }

    async fn revoke(&self, invitee_id: &str) -> Result<(), AppError> {
        revoke_sql(&self.pool, invitee_id).await
    }
}

// ── Validators ─────────────────────────────────────────────────────────

fn validate_mint_input(input: &MintInput) -> Result<(), AppError> {
    if input.label.trim().is_empty() {
        return Err(AppError::BadRequest("label required".into()));
    }
    if input.label.chars().count() > MAX_LABEL_LEN {
        return Err(AppError::BadRequest(format!(
            "label must be <= {MAX_LABEL_LEN} chars"
        )));
    }
    if matches!(input.tier, UserTier::Master) {
        return Err(AppError::BadRequest("tier must be pro or pro_max".into()));
    }
    validate_days(input.days)
}

fn validate_days(days: u32) -> Result<(), AppError> {
    if !ALLOWED_DAYS.contains(&days) {
        return Err(AppError::BadRequest(format!(
            "days must be one of {ALLOWED_DAYS:?}"
        )));
    }
    Ok(())
}

// ── Mint ───────────────────────────────────────────────────────────────

async fn mint_with_retry(pool: &PgPool, input: &MintInput) -> Result<MintOutput, AppError> {
    let mut last_err: Option<AppError> = None;
    for _ in 0..INSERT_MAX_ATTEMPTS {
        match try_insert_invite(pool, input).await {
            Ok(out) => return Ok(out),
            Err(AppError::Sqlx(e)) if crate::services::db::is_pg_duplicate_key(&e) => {
                last_err = Some(AppError::Sqlx(e));
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    Err(last_err.unwrap_or_else(|| AppError::Other(anyhow::anyhow!("mint retry exhausted"))))
}

async fn try_insert_invite(pool: &PgPool, input: &MintInput) -> Result<MintOutput, AppError> {
    let code = invite_codes::generate();
    let hint = invite_codes::hint(&code);
    let phc = code_hash::hash(&code)?;
    let id = new_user_id();
    let expires_at = Utc::now() + chrono::Duration::days(i64::from(input.days));
    sqlx::query(
        "INSERT INTO users \
         (id, code_hash, code_hint, label, tier, is_master, expires_at, created_at) \
         VALUES ($1, $2, $3, $4, $5, FALSE, $6, NOW())",
    )
    .bind(&id)
    .bind(&phc)
    .bind(&hint)
    .bind(input.label.trim())
    .bind(input.tier.as_str())
    .bind(expires_at)
    .execute(pool)
    .await?;
    tracing::info!(
        event = "invite.mint",
        invitee_id = %id,
        tier = %input.tier.as_str(),
        label = %input.label.trim(),
        days = input.days,
        code_hint = %hint,
        "invite minted",
    );
    Ok(MintOutput {
        id,
        plain_code: code,
        code_hint: hint,
        expires_at,
    })
}

/// 6-char lowercase-alphanum suffix, prefixed with `usr` to satisfy the
/// `users.id ~ '^usr[a-z0-9]{6}$'` CHECK constraint. Modulo bias on a
/// 36-char alphabet is ~0.4% per char — well below the collision budget
/// on a 36^6 space.
fn new_user_id() -> String {
    use rand::rngs::OsRng;
    use rand::RngCore;
    let mut raw = [0u8; 6];
    OsRng.fill_bytes(&mut raw);
    let mut suffix = [0u8; 6];
    for (i, b) in raw.iter().enumerate() {
        suffix[i] = ID_ALPHABET[(*b as usize) % ID_ALPHABET.len()];
    }
    let suffix = std::str::from_utf8(&suffix).expect("alphabet is ascii");
    format!("usr{suffix}")
}

// ── Extend / revoke / list ────────────────────────────────────────────

async fn extend_sql(pool: &PgPool, invitee_id: &str, days: u32) -> Result<DateTime<Utc>, AppError> {
    let interval = format!("{days} days");
    let new_expiry: Option<DateTime<Utc>> = sqlx::query_scalar(
        "UPDATE users \
         SET expires_at = GREATEST(NOW(), COALESCE(expires_at, NOW())) + $1::interval \
         WHERE id = $2 AND is_master = FALSE \
         RETURNING expires_at",
    )
    .bind(&interval)
    .bind(invitee_id)
    .fetch_optional(pool)
    .await?;
    let expires_at = new_expiry.ok_or_else(|| AppError::NotFound(format!("user {invitee_id}")))?;
    tracing::info!(
        event = "invite.extend",
        invitee_id = %invitee_id,
        days = days,
        "invite expires_at extended",
    );
    Ok(expires_at)
}

async fn revoke_sql(pool: &PgPool, invitee_id: &str) -> Result<(), AppError> {
    let rows = sqlx::query(
        "UPDATE users SET revoked_at = NOW() \
         WHERE id = $1 AND is_master = FALSE AND revoked_at IS NULL",
    )
    .bind(invitee_id)
    .execute(pool)
    .await?
    .rows_affected();
    if rows == 0 {
        return Err(AppError::NotFound(format!("user {invitee_id}")));
    }
    tracing::info!(
        event = "invite.revoke",
        invitee_id = %invitee_id,
        "invite revoked",
    );
    Ok(())
}

async fn list_invitees_sql(pool: &PgPool) -> Result<Vec<InviteeRow>, AppError> {
    let rows = sqlx::query!(
        r#"
        SELECT u.id, u.label, u.tier, u.code_hint,
               u.created_at, u.last_seen_at, u.activated_at,
               u.expires_at, u.revoked_at,
               COALESCE(used.used, 0)::BIGINT AS "used!: i64"
        FROM users u
        LEFT JOIN LATERAL (
            SELECT SUM(cost_units)::BIGINT AS used
            FROM request_log
            WHERE user_id = u.id
        ) used ON TRUE
        WHERE u.is_master = FALSE
        ORDER BY u.created_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            let tier = UserTier::parse(&r.tier).ok_or_else(|| {
                AppError::Other(anyhow::anyhow!("users.tier {:?} not in CHECK set", r.tier))
            })?;
            Ok(InviteeRow {
                id: r.id,
                label: r.label,
                tier,
                code_hint: r.code_hint,
                created_at: r.created_at,
                last_seen_at: r.last_seen_at,
                activated_at: r.activated_at,
                expires_at: r.expires_at,
                revoked_at: r.revoked_at,
                used_units: r.used.max(0) as u64,
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "invite_admin_tests.rs"]
mod tests;

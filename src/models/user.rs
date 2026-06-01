//! `users` table model — multi-user identity (Phase 1).
//!
//! Mirrors the Postgres schema in `migrations/0002_users.sql`:
//! - `is_master` plus the `users_master_consistency_chk` CHECK constraint
//!   pin tier=`master` and `expires_at IS NULL` together;
//! - `code_hash` is the argon2id hash; the raw invite code is never stored;
//! - `code_hint` is the user-visible "first 4 + … + last 2" hint shown next
//!   to the invite-code input on `/login` (Phase 1.2). Forensic only — does
//!   not weaken the brute-force search space meaningfully.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Tier taxonomy. Matches the `CHECK (tier IN (...))` clause in 0002.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UserTier {
    Pro,
    ProMax,
    Master,
}

impl UserTier {
    pub fn as_str(self) -> &'static str {
        match self {
            UserTier::Pro => "pro",
            UserTier::ProMax => "pro_max",
            UserTier::Master => "master",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pro" => Some(UserTier::Pro),
            "pro_max" => Some(UserTier::ProMax),
            "master" => Some(UserTier::Master),
            _ => None,
        }
    }
}

/// One row in the `users` table. `id` is `usrmaster` for the seeded master
/// user, otherwise `^usr[a-z0-9]{6}$`. Soft-delete is `revoked_at IS NOT NULL`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct User {
    pub id: String,
    pub code_hash: String,
    pub code_hint: String,
    pub label: String,
    pub tier: UserTier,
    pub is_master: bool,
    pub activated_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub invited_by: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl User {
    /// True iff this row currently grants access: not revoked, and either
    /// no expiry or an expiry still in the future. Routing layers should
    /// call this before honoring a session cookie.
    pub fn is_active(&self, now: DateTime<Utc>) -> bool {
        if self.revoked_at.is_some() {
            return false;
        }
        match self.expires_at {
            Some(exp) => exp > now,
            None => true,
        }
    }
}

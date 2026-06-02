//! Pitches — canonical answers to intro/narrative interview questions.
//!
//! A `Pitch` is the persistent shell for one intro-question type (e.g.
//! "Tell me about yourself"). Catalog data (`question_text`, `blurb`,
//! `sort_order`) lives on the `pitches` row (global, slug-id, seeded);
//! per-user state (`status`, `current_version_id`, `chat`) lives on
//! `pitch_user_state` keyed by `(owner_id, pitch_id)` — see migration
//! 0004. Reads do a LEFT JOIN so the in-memory `Pitch` keeps the unified
//! shape callers expect, while writes target each side independently
//! (catalog stays read-only; state UPSERTs into `pitch_user_state`).
//!
//! `PitchVersion` is an immutable snapshot of the locked-in spoken answer.
//! No bullets layer — intro answers are narrative, not structured incident
//! proofs, so the prose IS the artifact (short ≈ 90s, long ≈ 3min).

use serde::{Deserialize, Serialize};

use crate::services::id_gen::{self, Kind};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PitchStatus {
    NotStarted,
    InProgress,
    Locked,
}

impl PitchStatus {
    /// String form that mirrors the serde `rename_all = "snake_case"`
    /// mapping. Use this for Postgres CHECK literals
    /// (`pitch_user_state.status`) so the on-disk representation can never
    /// drift from the enum.
    pub fn as_str(self) -> &'static str {
        match self {
            PitchStatus::NotStarted => "not_started",
            PitchStatus::InProgress => "in_progress",
            PitchStatus::Locked => "locked",
        }
    }

    /// Inverse of [`as_str`]. Mirrors the CHECK in migration 0004.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "not_started" => Some(Self::NotStarted),
            "in_progress" => Some(Self::InProgress),
            "locked" => Some(Self::Locked),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pitch {
    /// Slug-as-id. Stable across re-seeds so URLs survive.
    /// Examples: "tell-me-about-yourself", "why-this-role".
    pub id: String,
    /// Canonical interviewer-facing phrasing of the question.
    pub question_text: String,
    /// One-line tile description. What this pitch is *for*.
    pub blurb: String,
    /// Display order on the tile grid. Lower = first.
    pub sort_order: i32,
    pub status: PitchStatus,
    /// FK → `pitch_versions.id` of the locked-in version. `None` until
    /// the user clicks Generate the first time.
    #[serde(default)]
    pub current_version_id: Option<String>,
    /// Embedded chat — same shape as `Story.chat`. Append-only.
    #[serde(default)]
    pub chat: Vec<crate::models::ChatTurn>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Immutable snapshot of one locked-in spoken answer. No `body` (no bullets
/// layer) — intro answers are narrative prose end-to-end.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PitchVersion {
    pub id: String,
    pub pitch_id: String,
    /// Monotonic per pitch: 1, 2, 3, …
    pub version_n: u32,
    /// ≈90s monologue. ~180–240 words.
    pub short: String,
    /// ≈3min fuller version. ~400–500 words.
    pub long: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl PitchVersion {
    /// Mint a fresh pitch-version id via the shared minter.
    pub fn new_id() -> String {
        id_gen::new(Kind::PitchVersion)
    }
}

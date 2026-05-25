//! Pitches — canonical answers to intro/narrative interview questions.
//!
//! A `Pitch` is the persistent shell for one intro-question type (e.g.
//! "Tell me about yourself"). Catalog data (question_text, blurb,
//! sort_order) is system-seeded; user state (status, chat, current version)
//! lives on the same row because the app is single-user — re-seed uses
//! `$setOnInsert` so it never clobbers state. When per-company variants
//! (#3) land, state will split into a separate collection keyed by
//! `(pitch_id, company_id)`.
//!
//! `PitchVersion` is an immutable snapshot of the locked-in spoken answer.
//! No bullets layer — intro answers are narrative, not structured incident
//! proofs, so the prose IS the artifact (short ≈ 90s, long ≈ 3min).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PitchStatus {
    NotStarted,
    InProgress,
    Locked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pitch {
    /// Slug-as-id. Stable across re-seeds so URLs survive.
    /// Examples: "tell-me-about-yourself", "why-this-role".
    #[serde(rename = "_id")]
    pub id: String,
    /// Canonical interviewer-facing phrasing of the question.
    pub question_text: String,
    /// One-line tile description. What this pitch is *for*.
    pub blurb: String,
    /// Display order on the tile grid. Lower = first.
    pub sort_order: i32,
    pub status: PitchStatus,
    /// FK → `pitch_versions._id` of the locked-in version. `None` until
    /// the user clicks Generate the first time.
    #[serde(default)]
    pub current_version_id: Option<String>,
    /// Embedded chat — same shape as `Story.chat`. Append-only.
    #[serde(default)]
    pub chat: Vec<crate::models::ChatTurn>,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl Pitch {
    pub const COLLECTION: &'static str = "pitches";
}

/// Immutable snapshot of one locked-in spoken answer. No `body` (no bullets
/// layer) — intro answers are narrative prose end-to-end.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PitchVersion {
    #[serde(rename = "_id")]
    pub id: String,
    pub pitch_id: String,
    /// Monotonic per pitch: 1, 2, 3, …
    pub version_n: u32,
    /// ≈90s monologue. ~180–240 words.
    pub short: String,
    /// ≈3min fuller version. ~400–500 words.
    pub long: String,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl PitchVersion {
    pub const COLLECTION: &'static str = "pitch_versions";
}

//! Story Builder — competency-tagged STAR+ stories driven by AI-led probing.
//!
//! A `Story` is the persistent shell: chosen competency, difficulty mode for
//! the coach, embedded chat history, and a pointer to the current
//! `StoryVersion`. `StoryVersion` is an immutable snapshot of the bullet-form
//! story at one point in time. Chat is the source of truth; bullets are the
//! index. The difficulty mode picks which `story_chat*` prompt drives the
//! coach for this story.

use serde::{Deserialize, Serialize};

use crate::services::id_gen::{self, Kind};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StoryStatus {
    InProgress,
    Complete,
}

impl StoryStatus {
    /// String form that mirrors the serde `rename_all = "snake_case"` mapping.
    /// Use this for Postgres CHECK literals (`stories.status`) so the on-disk
    /// representation can never drift from the enum.
    pub fn as_str(self) -> &'static str {
        match self {
            StoryStatus::InProgress => "in_progress",
            StoryStatus::Complete => "complete",
        }
    }

    /// Inverse of [`as_str`]. Mirrors the CHECK in migration 0001.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "in_progress" => Some(Self::InProgress),
            "complete" => Some(Self::Complete),
            _ => None,
        }
    }
}

/// Coach behavior for this story's chat. `Strict` is the default — pure
/// probing, never volunteers wording. `Collaborative` is the same probing
/// rigor with one relaxation: when the candidate explicitly asks for help,
/// ideas, or examples, the coach may offer 2–3 grounded options to pick
/// from or modify. The candidate's story still remains theirs.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Difficulty {
    #[default]
    Strict,
    Collaborative,
}

impl Difficulty {
    pub fn as_str(self) -> &'static str {
        match self {
            Difficulty::Strict => "strict",
            Difficulty::Collaborative => "collaborative",
        }
    }

    pub fn from_form(s: &str) -> Self {
        match s {
            "collaborative" => Difficulty::Collaborative,
            _ => Difficulty::Strict,
        }
    }

    /// Inverse of [`as_str`]. Mirrors the CHECK in migration 0001.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "strict" => Some(Self::Strict),
            "collaborative" => Some(Self::Collaborative),
            _ => None,
        }
    }

    pub fn prompt_name(self) -> &'static str {
        match self {
            Difficulty::Strict => "story_chat",
            Difficulty::Collaborative => "story_chat_collaborative",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    User,
    Assistant,
}

impl ChatRole {
    pub fn as_str(self) -> &'static str {
        match self {
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTurn {
    pub role: ChatRole,
    pub content: String,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub ts: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Story {
    #[serde(rename = "_id")]
    pub id: String,
    /// Owner of this story row. Set by the calling handler from
    /// `CurrentUser::id`. Legacy Mongo docs lack this field; the importer
    /// backfills before insert.
    #[serde(default)]
    pub owner_id: String,
    /// FK → `categories._id`. The competency this story is being built against.
    pub competency_id: String,
    pub status: StoryStatus,
    /// Picks the coach prompt for this story (`story_chat` or
    /// `story_chat_collaborative`). Defaults to `Strict` for legacy rows that
    /// pre-date the toggle, preserving previous behavior.
    #[serde(default)]
    pub mode: Difficulty,
    /// FK → `story_versions._id` of the latest locked-in version. `None` until
    /// the user clicks Generate the first time.
    #[serde(default)]
    pub current_version_id: Option<String>,
    /// Embedded chat — monotonic, append-only. Capped at
    /// [`crate::services::story_store::MAX_CHAT_TURNS`] turns.
    #[serde(default)]
    pub chat: Vec<ChatTurn>,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl Story {
    /// Legacy Mongo collection name. Retained for the one-shot importer.
    pub const COLLECTION: &'static str = "stories";

    /// Mint a fresh story id via the shared minter.
    pub fn new_id() -> String {
        id_gen::new(Kind::Story)
    }
}

/// STAR+ bullets per section. Each list is 3–6 short bullets in the
/// candidate's voice, generated by the summarize prompt.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StoryBody {
    #[serde(default)]
    pub situation: Vec<String>,
    #[serde(default)]
    pub task: Vec<String>,
    #[serde(default)]
    pub action: Vec<String>,
    #[serde(default)]
    pub result: Vec<String>,
    /// The "+" of STAR+ — reflection / what was learned / how it changed
    /// the candidate's approach.
    #[serde(default)]
    pub reflection: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryVersion {
    #[serde(rename = "_id")]
    pub id: String,
    pub story_id: String,
    /// Monotonic per story: 1, 2, 3, …
    pub version_n: u32,
    pub body: StoryBody,
    /// Spoken-prose variants generated on demand from `body`. `None` until the
    /// candidate clicks Generate; replaced wholesale on Regenerate. Kept on
    /// the version (rather than a separate doc) because the variants are a
    /// pure function of the bullets at this version — they share its identity
    /// and lifecycle.
    #[serde(default)]
    pub spoken: Option<SpokenStory>,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl StoryVersion {
    /// Legacy Mongo collection name. Retained for the one-shot importer.
    pub const COLLECTION: &'static str = "story_versions";

    /// Mint a fresh story-version id via the shared minter.
    pub fn new_id() -> String {
        id_gen::new(Kind::StoryVersion)
    }
}

/// Two spoken-prose variants of the same StoryBody: a 3–5 minute monologue
/// for time-constrained interviewers and a longer rehearsal-and-deliver
/// version. Both are first-person verbatim prose — no STAR+ headers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpokenStory {
    /// 3–5 min monologue. ~450–700 words.
    pub short: String,
    /// Longer fuller version. ~900–1400 words.
    pub long: String,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub generated_at: chrono::DateTime<chrono::Utc>,
}

#[cfg(test)]
#[path = "story_tests.rs"]
mod tests;

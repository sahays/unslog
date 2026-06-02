use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Book,
    Resume,
    Other,
}

impl AssetKind {
    /// Stable lowercase token used in form values and as the discriminator
    /// in the Postgres `kind` column. Must round-trip with `from_form` /
    /// `from_str` and match the CHECK constraint in `migrations/0001`.
    pub fn as_str(self) -> &'static str {
        match self {
            AssetKind::Book => "book",
            AssetKind::Resume => "resume",
            AssetKind::Other => "other",
        }
    }

    /// Inverse of [`as_str`]. Returns `None` for unknown values so callers
    /// can decide between defaulting and rejecting.
    pub fn from_form(s: &str) -> Option<Self> {
        Self::parse(s)
    }

    /// Parse the on-the-wire / on-the-row token back into a variant.
    /// `None` => unknown — the row-builder layer treats this as data
    /// corruption (CHECK constraint would have rejected the insert).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "book" => Some(AssetKind::Book),
            "resume" => Some(AssetKind::Resume),
            "other" => Some(AssetKind::Other),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionStatus {
    Pending,
    Ok,
    Failed,
}

impl ExtractionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ExtractionStatus::Pending => "pending",
            ExtractionStatus::Ok => "ok",
            ExtractionStatus::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(ExtractionStatus::Pending),
            "ok" => Some(ExtractionStatus::Ok),
            "failed" => Some(ExtractionStatus::Failed),
            _ => None,
        }
    }
}

/// Uploaded book / resume / other reference. Backed by Postgres `assets`.
/// `owner_id` scopes the row to a user; resumes are per-user, while
/// `kind = 'book'` rows are pinned to the master user by a trigger (the
/// critique prompt inlines the book as global content).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub id: String,
    pub owner_id: String,
    pub name: String,
    pub kind: AssetKind,
    pub primary: bool,
    pub original_filename: String,
    pub original_path: String,
    #[serde(default)]
    pub extracted_path: Option<String>,
    pub extraction_status: ExtractionStatus,
    #[serde(default)]
    pub extraction_error: Option<String>,
    pub uploaded_at: chrono::DateTime<chrono::Utc>,
}

impl Asset {
    pub fn new(
        owner_id: String,
        name: String,
        kind: AssetKind,
        original_filename: String,
        original_path: String,
    ) -> Self {
        Self {
            id: crate::services::id_gen::new(crate::services::id_gen::Kind::Asset),
            owner_id,
            name,
            kind,
            primary: false,
            original_filename,
            original_path,
            extracted_path: None,
            extraction_status: ExtractionStatus::Pending,
            extraction_error: None,
            uploaded_at: chrono::Utc::now(),
        }
    }
}

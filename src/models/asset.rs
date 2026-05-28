use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Book,
    Resume,
    Other,
}

impl AssetKind {
    /// Stable lowercase token used in form values and in the BSON-serialized
    /// kind discriminator. Must round-trip through `serde(rename_all =
    /// "snake_case")` above — keep the two in sync.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    #[serde(rename = "_id")]
    pub id: String,
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
    #[serde(with = "crate::models::datetime_compat::required")]
    pub uploaded_at: chrono::DateTime<chrono::Utc>,
}

impl Asset {
    pub const COLLECTION: &'static str = "assets";

    pub fn new(
        name: String,
        kind: AssetKind,
        original_filename: String,
        original_path: String,
    ) -> Self {
        Self {
            id: uuid::Uuid::now_v7().to_string(),
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

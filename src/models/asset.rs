use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Book,
    Other,
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
    pub uploaded_at: chrono::DateTime<chrono::Utc>,
}

impl Asset {
    pub const COLLECTION: &'static str = "assets";

    pub fn new(name: String, kind: AssetKind, original_filename: String, original_path: String) -> Self {
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

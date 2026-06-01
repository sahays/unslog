//! Row mapper for the `assets` table.
//!
//! The `kind` and `extraction_status` columns are TEXT + CHECK on the
//! Postgres side but typed enums on the domain side, so we can't use
//! `sqlx::query_as!()` directly on [`crate::models::Asset`]. This module
//! holds the raw row shape + the converter that fails fast when an
//! out-of-CHECK value sneaks through (data corruption / schema drift).

use crate::error::AppError;
use crate::models::{Asset, AssetKind, ExtractionStatus};

/// Columns selected by every read query, in the order [`AssetRow`]
/// expects. Centralized so a schema add only touches one place.
pub(super) const ASSET_COLS: &str = r#"id, owner_id, name, kind, "primary" AS is_primary,
    original_filename, original_path, extracted_path,
    extraction_status, extraction_error, uploaded_at"#;

/// Raw row shape — TEXT enums stay TEXT here and convert to [`Asset`] in
/// one place. Keeps the typed `AssetKind` / `ExtractionStatus` enums on
/// the domain side without a sqlx `Type` derive (the underlying column
/// is `TEXT` + CHECK, not a Postgres ENUM).
#[derive(sqlx::FromRow)]
pub(super) struct AssetRow {
    pub id: String,
    pub owner_id: String,
    pub name: String,
    pub kind: String,
    pub is_primary: bool,
    pub original_filename: String,
    pub original_path: String,
    pub extracted_path: Option<String>,
    pub extraction_status: String,
    pub extraction_error: Option<String>,
    pub uploaded_at: chrono::DateTime<chrono::Utc>,
}

impl AssetRow {
    pub(super) fn try_into_asset(self) -> Result<Asset, AppError> {
        let kind = AssetKind::parse(&self.kind).ok_or_else(|| {
            AppError::Other(anyhow::anyhow!(
                "assets.kind {:?} not in CHECK set",
                self.kind
            ))
        })?;
        let extraction_status =
            ExtractionStatus::parse(&self.extraction_status).ok_or_else(|| {
                AppError::Other(anyhow::anyhow!(
                    "assets.extraction_status {:?} not in CHECK set",
                    self.extraction_status
                ))
            })?;
        Ok(Asset {
            id: self.id,
            owner_id: self.owner_id,
            name: self.name,
            kind,
            primary: self.is_primary,
            original_filename: self.original_filename,
            original_path: self.original_path,
            extracted_path: self.extracted_path,
            extraction_status,
            extraction_error: self.extraction_error,
            uploaded_at: self.uploaded_at,
        })
    }
}

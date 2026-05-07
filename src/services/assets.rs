//! Asset upload + PDF text extraction.
//!
//! Strategy: try `pdf-extract` first (pure Rust, no system dep). If it errors
//! or returns suspiciously little text, fall back to shelling out to
//! `pdftotext` (poppler) when present. Output is a single Markdown-ish text
//! file written to `data/assets/extracted/<asset_id>.md`.

use std::path::{Path, PathBuf};
use tokio::process::Command;

use crate::error::AppError;
use crate::models::{Asset, AssetKind, ExtractionStatus};

const MIN_EXTRACTED_CHARS: usize = 200;

pub fn originals_dir(data_dir: &str) -> PathBuf {
    Path::new(data_dir).join("assets").join("originals")
}

pub fn extracted_dir(data_dir: &str) -> PathBuf {
    Path::new(data_dir).join("assets").join("extracted")
}

pub fn original_path_for(data_dir: &str, asset_id: &str, ext: &str) -> PathBuf {
    originals_dir(data_dir).join(format!("{asset_id}.{ext}"))
}

pub fn extracted_path_for(data_dir: &str, asset_id: &str) -> PathBuf {
    extracted_dir(data_dir).join(format!("{asset_id}.md"))
}

/// Persist an uploaded blob to `data/assets/originals/<id>.<ext>` and
/// build a freshly-minted `Asset` row (extraction_status: pending).
pub async fn save_upload(
    data_dir: &str,
    name: String,
    kind: AssetKind,
    original_filename: String,
    bytes: &[u8],
) -> Result<Asset, AppError> {
    let ext = Path::new(&original_filename)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("bin")
        .to_lowercase();

    let asset_id = uuid::Uuid::now_v7().to_string();
    tokio::fs::create_dir_all(originals_dir(data_dir)).await?;
    tokio::fs::create_dir_all(extracted_dir(data_dir)).await?;

    let path = original_path_for(data_dir, &asset_id, &ext);
    tokio::fs::write(&path, bytes).await?;

    let mut asset = Asset::new(
        name,
        kind,
        original_filename,
        path.to_string_lossy().into_owned(),
    );
    asset.id = asset_id;
    Ok(asset)
}

/// Run extraction for an already-saved asset, writing the extracted markdown
/// and returning the updated extraction_status / extracted_path / error.
pub async fn extract(
    data_dir: &str,
    asset: &Asset,
) -> (ExtractionStatus, Option<String>, Option<String>) {
    let original = PathBuf::from(&asset.original_path);
    let target = extracted_path_for(data_dir, &asset.id);

    let result = run_pdf_extract(&original).await;
    let text = match result {
        Ok(t) if t.chars().count() >= MIN_EXTRACTED_CHARS => Some(t),
        _ => match run_pdftotext(&original).await {
            Ok(t) if !t.trim().is_empty() => Some(t),
            Ok(_) => None,
            Err(_) => None,
        },
    };

    let Some(text) = text else {
        let err = "extraction returned no usable text (try pdftotext install or check the PDF)";
        return (ExtractionStatus::Failed, None, Some(err.to_string()));
    };

    if let Some(parent) = target.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Err(e) = tokio::fs::write(&target, text.as_bytes()).await {
        return (
            ExtractionStatus::Failed,
            None,
            Some(format!("failed to write extracted markdown: {e}")),
        );
    }

    (
        ExtractionStatus::Ok,
        Some(target.to_string_lossy().into_owned()),
        None,
    )
}

async fn run_pdf_extract(path: &Path) -> Result<String, String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || pdf_extract::extract_text(&path).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

async fn run_pdftotext(path: &Path) -> Result<String, String> {
    let output = Command::new("pdftotext")
        .arg("-layout")
        .arg(path)
        .arg("-")
        .output()
        .await
        .map_err(|e| format!("pdftotext not available: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "pdftotext exited with {}",
            output.status.code().unwrap_or(-1)
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}

/// Read the extracted text for an asset (returns "" if no extracted file).
pub async fn read_extracted(asset: &Asset) -> Result<String, AppError> {
    let Some(path) = asset.extracted_path.as_ref() else {
        return Ok(String::new());
    };
    Ok(tokio::fs::read_to_string(path).await.unwrap_or_default())
}

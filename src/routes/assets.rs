use askama::Template;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use axum_extra::extract::Multipart;

use crate::error::AppError;
use crate::filters; // Custom Askama filters used by templates below.
use crate::models::{Asset, AssetKind, ExtractionStatus};
use crate::services::assets as svc;
use crate::services::{asset_store, redact, text_validation};
use crate::startup::AppState;

/// Cap on the display name (the label shown in the library list).
const MAX_ASSET_NAME: usize = 200;
/// Per-file cap on uploaded asset PDFs. The global body-limit is 50 MB to
/// accommodate large audio uploads; assets get a tighter cap so a single
/// huge PDF can't fill disk quietly.
const MAX_ASSET_BYTES: usize = 30 * 1024 * 1024;
/// PDF magic bytes — the first 4 bytes of any PDF are `%PDF`.
const PDF_MAGIC: &[u8] = b"%PDF";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/assets", get(list).post(upload))
        .route("/assets/:id/primary", post(set_primary))
        .route("/assets/:id/reextract", post(reextract))
        .route("/assets/:id/delete", post(delete))
        .route("/assets/:id/preview", get(preview))
}

#[derive(Template)]
#[template(path = "assets/list.html")]
struct ListTemplate {
    books: Vec<Asset>,
    resumes: Vec<Asset>,
    others: Vec<Asset>,
    has_primary_book: bool,
}

async fn list(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let assets = asset_store::list_sorted(&state.db).await?;
    let mut books = Vec::new();
    let mut resumes = Vec::new();
    let mut others = Vec::new();
    for a in assets {
        match a.kind {
            AssetKind::Book => books.push(a),
            AssetKind::Resume => resumes.push(a),
            AssetKind::Other => others.push(a),
        }
    }
    let has_primary_book = books.iter().any(|a| a.primary);
    crate::error::render_html(ListTemplate {
        books,
        resumes,
        others,
        has_primary_book,
    })
}

async fn upload(State(state): State<AppState>, mut form: Multipart) -> Result<Response, AppError> {
    let mut name: Option<String> = None;
    let mut kind = AssetKind::Book;
    let mut filename: Option<String> = None;
    let mut bytes: Vec<u8> = Vec::new();

    while let Some(field) = form
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?
    {
        let fname = field.name().unwrap_or("").to_string();
        match fname.as_str() {
            "name" => {
                name = Some(field.text().await.unwrap_or_default());
            }
            "kind" => {
                let v = field.text().await.unwrap_or_default();
                // Default to Book on unknown values — preserves the legacy
                // "no kind field" behavior the upload form used to have.
                kind = AssetKind::from_form(&v).unwrap_or(AssetKind::Book);
            }
            "file" => {
                filename = field.file_name().map(|s| s.to_string());
                bytes = field
                    .bytes()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("file read: {e}")))?
                    .to_vec();
            }
            _ => {}
        }
    }

    let original_filename =
        filename.ok_or_else(|| AppError::BadRequest("no file uploaded".into()))?;
    // Display name falls back to the original filename, but either way we
    // sanitize so the rendered label can't smuggle controls/null/oversize.
    let display_name_raw = name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| original_filename.clone());
    let display_name =
        text_validation::sanitize_short(&display_name_raw, MAX_ASSET_NAME, "asset name")?;

    validate_pdf_upload(&bytes)?;

    let mut asset = svc::save_upload(
        &state.config.data_dir,
        display_name,
        kind,
        original_filename,
        &bytes,
    )
    .await?;

    // Run extraction synchronously — small enough for a single-user app.
    let (status, extracted_path, err) = svc::extract(&state.config.data_dir, &asset).await;
    asset.extraction_status = status;
    asset.extracted_path = extracted_path;
    asset.extraction_error = err;

    // If no primary asset of this kind exists yet, mark this one primary.
    // Keeps the "one primary per kind" invariant on bootstrap without
    // requiring a separate user action.
    if asset_store::find_primary_by_kind(&state.db, kind)
        .await?
        .is_none()
    {
        asset.primary = true;
    }
    asset_store::insert(&state.db, &asset).await?;
    if asset.primary {
        invalidate_kind_cache(&state, kind).await;
    }
    tracing::info!(
        event = "asset.upload",
        asset_id = %asset.id,
        name = %redact::preview(&asset.name, 80),
        original_filename = %redact::preview(&asset.original_filename, 80),
        bytes = bytes.len(),
        primary = asset.primary,
        extraction_status = ?asset.extraction_status,
        "asset uploaded",
    );

    Ok(Redirect::to("/assets").into_response())
}

async fn set_primary(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    // Load to learn the kind, so we invalidate the matching cache.
    let asset = asset_store::find_or_404(&state.db, &id).await?;
    asset_store::set_primary(&state.db, &id).await?;
    invalidate_kind_cache(&state, asset.kind).await;
    Ok(Redirect::to("/assets").into_response())
}

async fn reextract(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let asset = asset_store::find_or_404(&state.db, &id).await?;
    let (status, extracted_path, err) = svc::extract(&state.config.data_dir, &asset).await;
    asset_store::update_extraction(&state.db, &id, status, extracted_path, err).await?;
    if asset.primary {
        invalidate_kind_cache(&state, asset.kind).await;
    }
    Ok(Redirect::to("/assets").into_response())
}

async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let asset = asset_store::find_or_404(&state.db, &id).await?;

    let _ = tokio::fs::remove_file(&asset.original_path).await;
    if let Some(ext) = asset.extracted_path.as_ref() {
        let _ = tokio::fs::remove_file(ext).await;
    }
    // `asset_store::delete` promotes the next-most-recent peer of the same
    // kind to primary *before* deleting the current primary, so the
    // invariant "≥1 primary per non-empty kind" never breaks (worst case:
    // brief two-primary window within that kind, recoverable through
    // `find_primary_by_kind`).
    let was_primary = asset.primary;
    let kind = asset.kind;
    asset_store::delete(&state.db, &asset).await?;

    if was_primary {
        invalidate_kind_cache(&state, kind).await;
    }

    Ok(Redirect::to("/assets").into_response())
}

/// Route-side helper: invalidate whichever in-process cache covers `kind`.
/// `Other` has no cache today and is a no-op.
async fn invalidate_kind_cache(state: &AppState, kind: AssetKind) {
    match kind {
        AssetKind::Book => state.book_cache.invalidate().await,
        AssetKind::Resume => state.resume_cache.invalidate().await,
        AssetKind::Other => {}
    }
}

#[derive(Template)]
#[template(path = "assets/preview.html")]
struct PreviewTemplate {
    asset: Asset,
    body: String,
    truncated: bool,
}

async fn preview(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Html<String>, AppError> {
    let asset = asset_store::find_or_404(&state.db, &id).await?;

    let mut body = svc::read_extracted(&asset).await?;
    let truncated = body.chars().count() > 80_000;
    if truncated {
        body = body.chars().take(80_000).collect();
    }

    crate::error::render_html(PreviewTemplate {
        asset,
        body,
        truncated,
    })
}

pub fn fmt_status(s: ExtractionStatus) -> &'static str {
    match s {
        ExtractionStatus::Ok => "ok",
        ExtractionStatus::Pending => "pending",
        ExtractionStatus::Failed => "failed",
    }
}

/// Trust-boundary validator for an inbound asset upload. Three checks,
/// each protecting a distinct failure mode:
///   1. **Empty** — a zero-byte upload is never legitimate and writing it
///      would create a dead row that pdf-extract would then crash on.
///   2. **Oversize** — caps disk usage so a single huge PDF cannot fill
///      the data dir silently. The global body cap is 50 MB to allow
///      audio; assets get a tighter cap.
///   3. **Magic-byte** — defends against a `.pdf`-suffixed but non-PDF
///      file (deliberate or accidental). pdf-extract would fail anyway,
///      but with a less useful error and after disk has been touched.
fn validate_pdf_upload(bytes: &[u8]) -> Result<(), AppError> {
    if bytes.is_empty() {
        return Err(AppError::BadRequest("uploaded file is empty".into()));
    }
    if bytes.len() > MAX_ASSET_BYTES {
        return Err(AppError::BadRequest(format!(
            "asset is {} bytes — max {MAX_ASSET_BYTES}",
            bytes.len()
        )));
    }
    if !bytes.starts_with(PDF_MAGIC) {
        return Err(AppError::BadRequest(
            "uploaded file is not a PDF (missing %PDF header)".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "assets_tests.rs"]
mod tests;

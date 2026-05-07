//! Serves files from data/recordings/ with path-traversal protection.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use std::path::PathBuf;

use crate::error::AppError;
use crate::startup::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/recordings/*path", get(serve))
}

async fn serve(
    State(state): State<AppState>,
    Path(path): Path<String>,
) -> Result<Response, AppError> {
    let root = PathBuf::from(&state.config.data_dir).join("recordings");
    let candidate = root.join(&path);
    let canonical_root = tokio::fs::canonicalize(&root).await.unwrap_or(root.clone());
    let canonical_candidate = tokio::fs::canonicalize(&candidate)
        .await
        .map_err(|_| AppError::NotFound(format!("recording {path}")))?;
    if !canonical_candidate.starts_with(&canonical_root) {
        return Err(AppError::BadRequest("path traversal".into()));
    }

    let bytes = tokio::fs::read(&canonical_candidate).await?;
    let mime = mime_for(&canonical_candidate);
    Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, "private, max-age=3600")
        .body(Body::from(bytes))
        .map_err(|e| AppError::Other(anyhow::anyhow!(e)))
}

fn mime_for(p: &std::path::Path) -> &'static str {
    match p.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase().as_str() {
        "mp3" => "audio/mpeg",
        "webm" => "audio/webm",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "m4a" | "mp4" => "audio/mp4",
        _ => "application/octet-stream",
    }
}

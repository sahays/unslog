use askama::Template;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use axum_extra::extract::Multipart;
use futures::TryStreamExt;
use mongodb::options::FindOptions;

use crate::error::AppError;
use crate::models::{Asset, AssetKind, ExtractionStatus};
use crate::services::assets as svc;
use crate::startup::AppState;

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
    assets: Vec<Asset>,
    has_primary: bool,
}

async fn list(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let coll = crate::db::assets(&state.db);
    let opts = FindOptions::builder()
        .sort(bson::doc! { "uploaded_at": -1 })
        .build();
    let cursor = coll.find(bson::doc! {}).with_options(opts).await?;
    let assets: Vec<Asset> = cursor.try_collect().await?;
    let has_primary = assets.iter().any(|a| a.primary);
    let body = ListTemplate {
        assets,
        has_primary,
    }
    .render()
    .map_err(|e| AppError::Other(anyhow::anyhow!("template error: {e}")))?;
    Ok(Html(body))
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
                kind = match v.as_str() {
                    "book" => AssetKind::Book,
                    _ => AssetKind::Other,
                };
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
    let display_name = name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| original_filename.clone());

    if bytes.is_empty() {
        return Err(AppError::BadRequest("uploaded file is empty".into()));
    }

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

    // If this is the only asset, mark it primary.
    let coll = crate::db::assets(&state.db);
    let count = coll.count_documents(bson::doc! {}).await?;
    if count == 0 {
        asset.primary = true;
    }
    coll.insert_one(&asset).await?;
    tracing::info!(
        event = "asset.upload",
        asset_id = %asset.id,
        name = %asset.name,
        original_filename = %asset.original_filename,
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
    let coll = crate::db::assets(&state.db);
    coll.update_many(bson::doc! {}, bson::doc! { "$set": { "primary": false } })
        .await?;
    let res = coll
        .update_one(
            bson::doc! { "_id": &id },
            bson::doc! { "$set": { "primary": true } },
        )
        .await?;
    if res.matched_count == 0 {
        return Err(AppError::NotFound(format!("asset {id}")));
    }
    Ok(Redirect::to("/assets").into_response())
}

async fn reextract(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let coll = crate::db::assets(&state.db);
    let asset: Asset = coll
        .find_one(bson::doc! { "_id": &id })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("asset {id}")))?;

    let (status, extracted_path, err) = svc::extract(&state.config.data_dir, &asset).await;
    coll.update_one(
        bson::doc! { "_id": &id },
        bson::doc! {
            "$set": {
                "extraction_status": bson::to_bson(&status)?,
                "extracted_path": extracted_path,
                "extraction_error": err,
            }
        },
    )
    .await?;

    Ok(Redirect::to("/assets").into_response())
}

async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let coll = crate::db::assets(&state.db);
    let asset: Asset = coll
        .find_one(bson::doc! { "_id": &id })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("asset {id}")))?;

    let _ = tokio::fs::remove_file(&asset.original_path).await;
    if let Some(ext) = asset.extracted_path.as_ref() {
        let _ = tokio::fs::remove_file(ext).await;
    }
    coll.delete_one(bson::doc! { "_id": &id }).await?;

    if asset.primary {
        // pick a new primary deterministically: most recently uploaded
        if let Some(next) = coll
            .find_one(bson::doc! {})
            .with_options(
                mongodb::options::FindOneOptions::builder()
                    .sort(bson::doc! { "uploaded_at": -1 })
                    .build(),
            )
            .await?
        {
            coll.update_one(
                bson::doc! { "_id": &next.id },
                bson::doc! { "$set": { "primary": true } },
            )
            .await?;
        }
    }

    Ok(Redirect::to("/assets").into_response())
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
    let coll = crate::db::assets(&state.db);
    let asset: Asset = coll
        .find_one(bson::doc! { "_id": &id })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("asset {id}")))?;

    let mut body = svc::read_extracted(&asset).await?;
    let truncated = body.chars().count() > 80_000;
    if truncated {
        body = body.chars().take(80_000).collect();
    }

    let body = PreviewTemplate {
        asset,
        body,
        truncated,
    }
    .render()
    .map_err(|e| AppError::Other(anyhow::anyhow!("template error: {e}")))?;
    Ok(Html(body))
}

pub fn fmt_status(s: ExtractionStatus) -> &'static str {
    match s {
        ExtractionStatus::Ok => "ok",
        ExtractionStatus::Pending => "pending",
        ExtractionStatus::Failed => "failed",
    }
}

//! OpenRouter `/models` listing with a process-local 1h cache.
//!
//! This is the data we feed into the Settings dropdowns. We don't try to
//! perfectly classify which models are "STT-capable" vs "chat-only" — we
//! filter on the architecture.input_modalities / output_modalities fields
//! when those are present, and otherwise show everything.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::sync::Mutex;

use crate::error::AppError;
use crate::services::openrouter::LlmClient;

const CACHE_TTL: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub architecture: Architecture,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Architecture {
    #[serde(default)]
    pub input_modalities: Vec<String>,
    #[serde(default)]
    pub output_modalities: Vec<String>,
}

impl ModelInfo {
    pub fn supports_audio_in(&self) -> bool {
        self.architecture
            .input_modalities
            .iter()
            .any(|m| m == "audio")
    }
    pub fn supports_audio_out(&self) -> bool {
        self.architecture
            .output_modalities
            .iter()
            .any(|m| m == "audio")
    }
    pub fn supports_text_chat(&self) -> bool {
        let im = &self.architecture.input_modalities;
        let om = &self.architecture.output_modalities;
        if im.is_empty() && om.is_empty() {
            return true;
        }
        im.iter().any(|m| m == "text") && om.iter().any(|m| m == "text")
    }
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelInfo>,
}

/// Cache shared across handlers.
#[derive(Clone, Default)]
pub struct ModelsCache {
    inner: Arc<Mutex<Option<CacheEntry>>>,
}

struct CacheEntry {
    fetched_at: Instant,
    models: Vec<ModelInfo>,
}

impl ModelsCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns models, refreshing if the cache is empty or expired. On a
    /// network error with no prior cache, surfaces the error; if there's a
    /// stale cache we keep using it and log the failure.
    pub async fn get(&self, or: &dyn LlmClient) -> Result<Vec<ModelInfo>, AppError> {
        {
            let guard = self.inner.lock().await;
            if let Some(entry) = guard.as_ref() {
                if entry.fetched_at.elapsed() < CACHE_TTL {
                    return Ok(entry.models.clone());
                }
            }
        }

        match fetch_models(or).await {
            Ok(models) => {
                let mut guard = self.inner.lock().await;
                *guard = Some(CacheEntry {
                    fetched_at: Instant::now(),
                    models: models.clone(),
                });
                Ok(models)
            }
            Err(e) => {
                let guard = self.inner.lock().await;
                if let Some(entry) = guard.as_ref() {
                    tracing::warn!(error = %e, "openrouter /models refresh failed; using stale cache");
                    return Ok(entry.models.clone());
                }
                Err(e)
            }
        }
    }

    /// Force-refresh on next read.
    pub async fn invalidate(&self) {
        let mut guard = self.inner.lock().await;
        *guard = None;
    }
}

async fn fetch_models(or: &dyn LlmClient) -> Result<Vec<ModelInfo>, AppError> {
    let raw = or.list_models_raw().await?;
    let parsed: ModelsResponse = serde_json::from_value(raw)
        .map_err(|e| AppError::Upstream(format!("/models parse: {e}")))?;
    let mut models = parsed.data;
    models.sort_by(|a, b| a.id.to_lowercase().cmp(&b.id.to_lowercase()));
    Ok(models)
}

//! `GET /models` — list available models. Separated from the four call
//! shapes (chat/tts/stt) since it's a flat info endpoint, not a generation
//! endpoint, and only the Settings page reads it.

use std::time::Instant;

use super::{OpenRouter, BASE_URL};
use crate::error::AppError;

impl OpenRouter {
    /// List available models from OpenRouter. Returns the raw `data` array
    /// JSON so callers can parse into their own shape.
    pub async fn list_models_raw(&self) -> Result<serde_json::Value, AppError> {
        let url = format!("{BASE_URL}/models");
        let start = Instant::now();
        let resp = self.auth_get(&url)?.send().await?;
        let duration_ms = start.elapsed().as_millis() as u64;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            tracing::warn!(
                op = "openrouter.list_models",
                http_status = status.as_u16(),
                duration_ms,
                body_preview = %crate::services::redact::preview(&text, 240),
                "openrouter /models failed",
            );
            return Err(AppError::Upstream(format!(
                "openrouter list_models failed: {status}"
            )));
        }
        let value = resp.json::<serde_json::Value>().await?;
        let count = value
            .get("data")
            .and_then(|d| d.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        tracing::info!(
            op = "openrouter.list_models",
            duration_ms,
            count,
            "openrouter /models ok",
        );
        Ok(value)
    }
}

//! `GET /models` — list available models. Separated from the four call
//! shapes (chat/tts/stt) since it's a flat info endpoint, not a generation
//! endpoint, and only the Settings page reads it.
//!
//! OpenRouter quirk: the bare `/models` response omits TTS providers
//! (mistralai/voxtral-mini-tts, microsoft/mai-voice-2, …). Those are only
//! returned when the call is scoped with `?output_modalities=speech`. We
//! issue both calls here and union the results so callers get a single
//! complete list and don't have to know the quirk.

use std::collections::BTreeMap;
use std::time::Instant;

use super::{OpenRouter, BASE_URL};
use crate::error::AppError;

impl OpenRouter {
    /// List available models from OpenRouter, unioning the bare list with
    /// the speech-filtered list (see module docs). Returns an object with a
    /// `data` array so callers can parse into their own shape. The speech
    /// fetch is best-effort: if it fails we log and fall back to the bare
    /// list rather than failing the whole settings page.
    pub async fn list_models_raw(&self) -> Result<serde_json::Value, AppError> {
        let bare = fetch_models_page(self, None).await?;
        let speech = match fetch_models_page(self, Some("speech")).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    op = "openrouter.list_models.speech",
                    error = %e,
                    "speech-filtered /models fetch failed; chat/STT list still usable",
                );
                serde_json::json!({ "data": [] })
            }
        };
        Ok(union_by_id(bare, speech))
    }
}

async fn fetch_models_page(
    or: &OpenRouter,
    output_modalities: Option<&str>,
) -> Result<serde_json::Value, AppError> {
    let url = match output_modalities {
        Some(m) => format!("{BASE_URL}/models?output_modalities={m}"),
        None => format!("{BASE_URL}/models"),
    };
    let start = Instant::now();
    let resp = or.auth_get(&url)?.send().await?;
    let duration_ms = start.elapsed().as_millis() as u64;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        tracing::warn!(
            op = "openrouter.list_models",
            output_modalities = output_modalities.unwrap_or(""),
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
        output_modalities = output_modalities.unwrap_or(""),
        duration_ms,
        count,
        "openrouter /models ok",
    );
    Ok(value)
}

/// Merge two `{data: [...]}` payloads, de-duping by `id`. Entries in `a`
/// win over `b` on collision (the bare list has richer pricing/context
/// metadata than the speech-filtered list).
fn union_by_id(a: serde_json::Value, b: serde_json::Value) -> serde_json::Value {
    let mut by_id: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    for source in [b, a] {
        if let Some(items) = source.get("data").and_then(|d| d.as_array()) {
            for item in items {
                if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                    by_id.insert(id.to_string(), item.clone());
                }
            }
        }
    }
    serde_json::json!({ "data": by_id.into_values().collect::<Vec<_>>() })
}

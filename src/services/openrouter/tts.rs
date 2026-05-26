//! Text-to-speech via OpenRouter's `/audio/speech`.

use std::time::Instant;

use super::{retry_once, OpenRouter, BASE_URL};
use crate::error::AppError;

impl OpenRouter {
    /// Synthesize speech. Returns the raw audio bytes (mp3 by default).
    /// `instructions` is the natural-language voice-steering field accepted by
    /// gpt-4o-mini-tts (e.g. "Speak with a British English accent."); empty
    /// string omits it. Other TTS models silently ignore it. Retries once on
    /// transient errors.
    pub async fn tts(
        &self,
        model: &str,
        voice: &str,
        text: &str,
        speed: Option<f32>,
        instructions: &str,
    ) -> Result<bytes::Bytes, AppError> {
        retry_once("openrouter.tts", || {
            self.tts_once(model, voice, text, speed, instructions)
        })
        .await
    }

    async fn tts_once(
        &self,
        model: &str,
        voice: &str,
        text: &str,
        speed: Option<f32>,
        instructions: &str,
    ) -> Result<bytes::Bytes, AppError> {
        let url = format!("{BASE_URL}/audio/speech");
        let input_chars = text.chars().count();
        let body = build_tts_body(model, voice, text, speed, instructions);
        let start = Instant::now();
        let resp = self.auth_post(&url)?.json(&body).send().await?;
        let duration_ms = start.elapsed().as_millis() as u64;

        if !resp.status().is_success() {
            let status = resp.status();
            let text_body = resp.text().await.unwrap_or_default();
            tracing::warn!(
                op = "openrouter.tts",
                model,
                voice,
                http_status = status.as_u16(),
                duration_ms,
                input_chars,
                body_preview = %crate::services::redact::preview(&text_body, 240),
                "openrouter tts failed",
            );
            return Err(AppError::Upstream(format!(
                "openrouter tts failed: {status}"
            )));
        }
        let bytes = resp.bytes().await?;
        tracing::info!(
            op = "openrouter.tts",
            model,
            voice,
            duration_ms,
            input_chars,
            audio_bytes = bytes.len(),
            "openrouter tts ok",
        );
        Ok(bytes)
    }
}

fn build_tts_body(
    model: &str,
    voice: &str,
    text: &str,
    speed: Option<f32>,
    instructions: &str,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "voice": voice,
        "input": text,
        "response_format": "mp3",
    });
    if let Some(s) = speed {
        body["speed"] = serde_json::json!(s);
    }
    if !instructions.is_empty() {
        body["instructions"] = serde_json::json!(instructions);
    }
    body
}

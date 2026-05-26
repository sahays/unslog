//! Speech-to-text via chat-completions with an `input_audio` content block.

use std::time::Instant;

use super::chat::first_choice;
use super::{retry_once, ChatCompletion, OpenRouter, BASE_URL};
use crate::error::AppError;

impl OpenRouter {
    /// Speech-to-text via chat-with-input_audio. Sends audio base64-encoded
    /// and asks the model to transcribe verbatim. Returns the model's text
    /// output, which we treat as the transcript after a light scrub.
    /// Retries once on transient body-decode / network errors — same pattern
    /// as `chat`/`tts`.
    pub async fn stt(
        &self,
        model: &str,
        audio_bytes: &[u8],
        format: &str,
    ) -> Result<String, AppError> {
        retry_once("openrouter.stt", || {
            self.stt_once(model, audio_bytes, format)
        })
        .await
    }

    async fn stt_once(
        &self,
        model: &str,
        audio_bytes: &[u8],
        format: &str,
    ) -> Result<String, AppError> {
        let audio_len = audio_bytes.len();
        let body = build_stt_body(model, audio_bytes, format);
        let url = format!("{BASE_URL}/chat/completions");
        let start = Instant::now();
        let resp = self.auth_post(&url)?.json(&body).send().await?;
        let duration_ms = start.elapsed().as_millis() as u64;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            tracing::warn!(
                op = "openrouter.stt",
                model,
                http_status = status.as_u16(),
                duration_ms,
                audio_bytes = audio_len,
                audio_format = format,
                body_preview = %crate::services::redact::preview(&text, 240),
                "openrouter stt failed",
            );
            return Err(AppError::Upstream(format!(
                "openrouter stt failed: {status}"
            )));
        }
        let parsed: ChatCompletion = resp.json().await?;
        let choice = first_choice(parsed, "openrouter stt")?;
        let transcript_chars = choice.message.content.chars().count();
        tracing::info!(
            op = "openrouter.stt",
            model,
            duration_ms,
            audio_bytes = audio_len,
            audio_format = format,
            transcript_chars,
            "openrouter stt ok",
        );
        Ok(choice.message.content)
    }
}

fn build_stt_body(model: &str, audio_bytes: &[u8], format: &str) -> serde_json::Value {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    let b64 = STANDARD.encode(audio_bytes);
    serde_json::json!({
        "model": model,
        "messages": [
            {
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": "Transcribe the following audio verbatim. Return only the transcript text — no preamble, no commentary, no quotes around it."
                    },
                    {
                        "type": "input_audio",
                        "input_audio": {
                            "data": b64,
                            "format": format,
                        }
                    }
                ]
            }
        ],
    })
}

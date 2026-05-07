//! Thin OpenRouter client. One client, four shapes of call:
//! 1. Chat completion (text in, text out) — used for critique, summary
//! 2. Chat completion with web tools — used for research agent (model:online)
//! 3. Audio in (base64 input_audio) — used for STT
//! 4. /audio/speech — used for TTS
//!
//! All four go through `reqwest::Client` with the OPENROUTER_API_KEY bearer.

use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AppError;

const BASE_URL: &str = "https://openrouter.ai/api/v1";

/// Default models — Phase 12 (settings) will let the user override.
pub const DEFAULT_CRITIQUE_MODEL: &str = "google/gemini-2.5-pro";
pub const DEFAULT_SUMMARY_MODEL: &str = "google/gemini-2.5-pro";
pub const DEFAULT_RESEARCH_MODEL: &str = "google/gemini-2.5-pro:online";
pub const DEFAULT_STT_MODEL: &str = "google/gemini-2.5-pro";
pub const DEFAULT_TTS_MODEL: &str = "openai/gpt-4o-mini-tts-2025-12-15";
pub const DEFAULT_TTS_VOICE: &str = "alloy";

#[derive(Clone)]
pub struct OpenRouter {
    http: reqwest::Client,
    api_key: String,
}

impl OpenRouter {
    pub fn new(http: reqwest::Client, api_key: String) -> Self {
        Self { http, api_key }
    }

    pub fn configured(&self) -> bool {
        !self.api_key.trim().is_empty()
    }

    fn require_key(&self) -> Result<&str, AppError> {
        if self.api_key.trim().is_empty() {
            Err(AppError::OpenRouterNotConfigured)
        } else {
            Ok(&self.api_key)
        }
    }

    /// List available models from OpenRouter. Returns the raw `data` array
    /// JSON so callers can parse into their own shape.
    pub async fn list_models_raw(&self) -> Result<serde_json::Value, AppError> {
        let key = self.require_key()?;
        let url = format!("{BASE_URL}/models");
        let start = Instant::now();
        let resp = self
            .http
            .get(&url)
            .bearer_auth(key)
            .header("HTTP-Referer", "https://github.com/sahays/unslog")
            .header("X-Title", "unslog")
            .send()
            .await?;
        let duration_ms = start.elapsed().as_millis() as u64;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            tracing::warn!(
                op = "openrouter.list_models",
                http_status = status.as_u16(),
                duration_ms,
                "openrouter /models failed",
            );
            return Err(AppError::Upstream(format!(
                "openrouter /models {status}: {text}"
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

    /// Plain chat completion. `messages` and `model` are caller-controlled.
    /// `force_json` adds `response_format: { type: "json_object" }`.
    pub async fn chat(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        force_json: bool,
    ) -> Result<String, AppError> {
        let key = self.require_key()?;

        let prompt_chars: usize = messages.iter().map(|m| m.content.chars().count()).sum();

        let mut body = serde_json::json!({
            "model": model,
            "messages": messages,
        });
        if force_json {
            body["response_format"] = serde_json::json!({ "type": "json_object" });
        }

        let url = format!("{BASE_URL}/chat/completions");
        let start = Instant::now();
        let resp = self
            .http
            .post(&url)
            .bearer_auth(key)
            .header("HTTP-Referer", "https://github.com/sahays/unslog")
            .header("X-Title", "unslog")
            .json(&body)
            .send()
            .await?;
        let duration_ms = start.elapsed().as_millis() as u64;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            tracing::warn!(
                op = "openrouter.chat",
                model,
                http_status = status.as_u16(),
                duration_ms,
                prompt_chars,
                "openrouter chat failed",
            );
            return Err(AppError::Upstream(format!(
                "openrouter chat {status}: {text}"
            )));
        }

        let parsed: ChatCompletion = resp.json().await?;
        let choice = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| AppError::Upstream("openrouter returned no choices".into()))?;
        let response_chars = choice.message.content.chars().count();
        tracing::info!(
            op = "openrouter.chat",
            model,
            duration_ms,
            prompt_chars,
            response_chars,
            force_json,
            "openrouter chat ok",
        );
        Ok(choice.message.content)
    }

    /// Synthesize speech. Returns the raw audio bytes (mp3 by default).
    pub async fn tts(
        &self,
        model: &str,
        voice: &str,
        text: &str,
        speed: Option<f32>,
    ) -> Result<bytes::Bytes, AppError> {
        let key = self.require_key()?;
        let url = format!("{BASE_URL}/audio/speech");
        let input_chars = text.chars().count();

        let mut body = serde_json::json!({
            "model": model,
            "voice": voice,
            "input": text,
            "response_format": "mp3",
        });
        if let Some(s) = speed {
            body["speed"] = serde_json::json!(s);
        }

        let start = Instant::now();
        let resp = self
            .http
            .post(&url)
            .bearer_auth(key)
            .header("HTTP-Referer", "https://github.com/sahays/unslog")
            .header("X-Title", "unslog")
            .json(&body)
            .send()
            .await?;
        let duration_ms = start.elapsed().as_millis() as u64;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            tracing::warn!(
                op = "openrouter.tts",
                model,
                voice,
                http_status = status.as_u16(),
                duration_ms,
                input_chars,
                "openrouter tts failed",
            );
            return Err(AppError::Upstream(format!(
                "openrouter tts {status}: {text}"
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

    /// Speech-to-text via chat-with-input_audio. Sends audio base64-encoded
    /// and asks the model to transcribe verbatim. Returns the model's text
    /// output, which we treat as the transcript after a light scrub.
    pub async fn stt(
        &self,
        model: &str,
        audio_bytes: &[u8],
        format: &str,
    ) -> Result<String, AppError> {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        let key = self.require_key()?;
        let audio_len = audio_bytes.len();

        let b64 = STANDARD.encode(audio_bytes);
        let url = format!("{BASE_URL}/chat/completions");

        let body = serde_json::json!({
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
        });

        let start = Instant::now();
        let resp = self
            .http
            .post(&url)
            .bearer_auth(key)
            .header("HTTP-Referer", "https://github.com/sahays/unslog")
            .header("X-Title", "unslog")
            .json(&body)
            .send()
            .await?;
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
                "openrouter stt failed",
            );
            return Err(AppError::Upstream(format!(
                "openrouter stt {status}: {text}"
            )));
        }

        let parsed: ChatCompletion = resp.json().await?;
        let choice =
            parsed.choices.into_iter().next().ok_or_else(|| {
                AppError::Upstream("openrouter returned no choices for stt".into())
            })?;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ChatCompletion {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct ChatChoiceMessage {
    content: String,
}

/// Pull a fenced JSON block (``` ... ``` or ```json ... ```) out of model output,
/// or fall through to raw content. Some models wrap JSON in fences even when
/// asked not to; this normalizes that.
pub fn unwrap_fenced_json(s: &str) -> &str {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("```json") {
        if let Some(end) = rest.rfind("```") {
            return rest[..end].trim();
        }
    }
    if let Some(rest) = s.strip_prefix("```") {
        if let Some(end) = rest.rfind("```") {
            return rest[..end].trim();
        }
    }
    s
}

pub fn parse_json<T: for<'de> Deserialize<'de>>(s: &str) -> Result<T, AppError> {
    let inner = unwrap_fenced_json(s);
    let v: Value = serde_json::from_str(inner)?;
    Ok(serde_json::from_value(v)?)
}

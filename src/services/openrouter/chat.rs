//! Chat completion calls — plain text + web-search variant.

use std::time::Instant;

use serde::Deserialize;

use super::{retry_once, ChatCompletion, ChatMessage, OpenRouter, BASE_URL};
use crate::error::AppError;

impl OpenRouter {
    /// Plain chat completion. `messages` and `model` are caller-controlled.
    /// `force_json` adds `response_format: { type: "json_object" }`.
    /// Retries once on transient body-decode / network errors before failing
    /// — same pattern as TTS, motivated by occasional reqwest "error decoding
    /// response body" mid-stream cutoffs from OpenRouter.
    pub async fn chat(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        force_json: bool,
    ) -> Result<String, AppError> {
        retry_once("openrouter.chat", || {
            self.chat_once(model, messages.clone(), force_json)
        })
        .await
    }

    async fn chat_once(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        force_json: bool,
    ) -> Result<String, AppError> {
        let prompt_chars: usize = messages.iter().map(|m| m.content.chars().count()).sum();
        let body = build_chat_body(model, &messages, force_json, false);
        let url = format!("{BASE_URL}/chat/completions");
        let start = Instant::now();
        let resp = self.auth_post(&url)?.json(&body).send().await?;
        let duration_ms = start.elapsed().as_millis() as u64;
        if !resp.status().is_success() {
            return Err(log_upstream_failure(
                "openrouter.chat",
                model,
                prompt_chars,
                duration_ms,
                resp,
            )
            .await);
        }
        let parsed: ChatCompletion = resp.json().await?;
        let choice = first_choice(parsed, "openrouter chat")?;
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

    /// Like [`chat`], but enables OpenRouter's `web` plugin (and appends
    /// `:online` to the model id is the caller's job — we add the plugin in
    /// the body so search runs even if the suffix was omitted). Returns the
    /// model's text plus the URLs the plugin actually fetched (from
    /// `message.annotations`). Empty `fetched_urls` is the loud signal that
    /// search did NOT run — callers should warn the user when that happens.
    /// Retries once on transient errors, same pattern as [`chat`].
    pub async fn chat_with_web_search(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        force_json: bool,
    ) -> Result<ChatWithWebSearchOut, AppError> {
        retry_once("openrouter.chat_web", || {
            self.chat_with_web_search_once(model, messages.clone(), force_json)
        })
        .await
    }

    async fn chat_with_web_search_once(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        force_json: bool,
    ) -> Result<ChatWithWebSearchOut, AppError> {
        let prompt_chars: usize = messages.iter().map(|m| m.content.chars().count()).sum();
        let body = build_chat_body(model, &messages, force_json, true);
        let url = format!("{BASE_URL}/chat/completions");
        let start = Instant::now();
        let resp = self.auth_post(&url)?.json(&body).send().await?;
        let duration_ms = start.elapsed().as_millis() as u64;
        if !resp.status().is_success() {
            return Err(log_upstream_failure(
                "openrouter.chat_web",
                model,
                prompt_chars,
                duration_ms,
                resp,
            )
            .await);
        }
        let parsed: ChatCompletion = resp.json().await?;
        let choice = first_choice(parsed, "openrouter chat_web")?;
        let response_chars = choice.message.content.chars().count();
        let fetched_urls = extract_fetched_urls(&choice.message.annotations);
        tracing::info!(
            op = "openrouter.chat_web",
            model,
            duration_ms,
            prompt_chars,
            response_chars,
            force_json,
            fetched_urls_n = fetched_urls.len(),
            "openrouter chat_with_web_search ok",
        );
        Ok(ChatWithWebSearchOut {
            content: choice.message.content,
            fetched_urls,
        })
    }
}

fn build_chat_body(
    model: &str,
    messages: &[ChatMessage],
    force_json: bool,
    with_web_plugin: bool,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
    });
    if with_web_plugin {
        body["plugins"] = serde_json::json!([ { "id": "web" } ]);
    }
    if force_json {
        body["response_format"] = serde_json::json!({ "type": "json_object" });
    }
    body
}

/// First choice out of a parsed completion or an upstream error.
pub(crate) fn first_choice(
    parsed: ChatCompletion,
    op: &str,
) -> Result<super::ChatChoice, AppError> {
    parsed
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| AppError::Upstream(format!("{op} returned no choices")))
}

/// Standard upstream-failure log line + `AppError::Upstream`. Consumes the
/// response (reads the body for the redacted preview), so call only on the
/// failure branch.
pub(crate) async fn log_upstream_failure(
    op: &'static str,
    model: &str,
    prompt_chars: usize,
    duration_ms: u64,
    resp: reqwest::Response,
) -> AppError {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    tracing::warn!(
        op = %op,
        model,
        http_status = status.as_u16(),
        duration_ms,
        prompt_chars,
        body_preview = %crate::services::redact::preview(&text, 240),
        "openrouter call failed",
    );
    AppError::Upstream(format!("{op} failed: {status}"))
}

#[derive(Debug, Deserialize)]
pub(crate) struct Annotation {
    #[serde(default)]
    pub(crate) url_citation: Option<UrlCitation>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UrlCitation {
    #[serde(default)]
    pub(crate) url: String,
}

/// Returned by [`chat_with_web_search`]. `fetched_urls` comes from OpenRouter's
/// `message.annotations` — these are the URLs the web plugin actually
/// retrieved, not anything the model produced in its body. Empty list ⇒ search
/// didn't run for this request.
#[derive(Debug, Clone)]
pub struct ChatWithWebSearchOut {
    pub content: String,
    pub fetched_urls: Vec<String>,
}

/// Flatten `message.annotations[*].url_citation.url` into a deduped, order-
/// preserving list. Skips annotations missing a citation and empty URLs.
pub(crate) fn extract_fetched_urls(anns: &[Annotation]) -> Vec<String> {
    use std::collections::HashSet;
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::with_capacity(anns.len());
    for a in anns {
        if let Some(uc) = &a.url_citation {
            if !uc.url.is_empty() && seen.insert(uc.url.clone()) {
                out.push(uc.url.clone());
            }
        }
    }
    out
}

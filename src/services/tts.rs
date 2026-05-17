//! TTS wrapper. Render text to MP3, save to disk, return relative path.

use std::path::PathBuf;

use crate::error::AppError;
use crate::services::openrouter::LlmClient;

#[allow(clippy::too_many_arguments)]
pub async fn synthesize(
    or: &dyn LlmClient,
    model: &str,
    voice: &str,
    text: &str,
    speed: Option<f32>,
    instructions: &str,
    out: PathBuf,
) -> Result<PathBuf, AppError> {
    if let Some(parent) = out.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let bytes = or.tts(model, voice, text, speed, instructions).await?;
    tokio::fs::write(&out, &bytes).await?;
    Ok(out)
}

/// Build an `instructions` string for gpt-4o-mini-tts from a BCP-47 locale
/// (`en-US`, `en-GB`, `en-IN`, `en-AU`). Returns an empty string for empty /
/// unknown codes so the caller can pass it straight through — TTS treats
/// empty as "no instructions, use model defaults".
pub fn build_accent_instructions(language: &str) -> String {
    let phrase = match language {
        "en-US" => "American English",
        "en-GB" => "British English",
        "en-IN" => "Indian English",
        "en-AU" => "Australian English",
        _ => return String::new(),
    };
    format!("Speak clearly with a {phrase} accent.")
}

#[cfg(test)]
#[path = "tts_tests.rs"]
mod tests;

//! TTS wrapper. Render text to MP3, save to disk, return relative path.

use std::path::PathBuf;

use crate::error::AppError;
use crate::services::openrouter::LlmClient;

pub async fn synthesize(
    or: &dyn LlmClient,
    model: &str,
    voice: &str,
    text: &str,
    speed: Option<f32>,
    out: PathBuf,
) -> Result<PathBuf, AppError> {
    if let Some(parent) = out.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let bytes = or.tts(model, voice, text, speed).await?;
    tokio::fs::write(&out, &bytes).await?;
    Ok(out)
}

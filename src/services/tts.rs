//! TTS wrapper. Render text to MP3, save to disk, return relative path.

use std::path::PathBuf;

use crate::error::AppError;
use crate::services::openrouter::OpenRouter;

pub async fn synthesize(
    or: &OpenRouter,
    model: &str,
    voice: &str,
    text: &str,
    out: PathBuf,
) -> Result<PathBuf, AppError> {
    if let Some(parent) = out.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let bytes = or.tts(model, voice, text, None).await?;
    tokio::fs::write(&out, &bytes).await?;
    Ok(out)
}

//! STT wrapper. Save audio to disk + call OpenRouter chat-with-input_audio,
//! return (relative_path, transcript).

use std::path::PathBuf;

use crate::error::AppError;
use crate::services::openrouter::LlmClient;

pub fn audio_path_for(
    data_dir: &str,
    company_id: &str,
    session_id: &str,
    uuid: &str,
    ext: &str,
) -> PathBuf {
    crate::recordings::session_dir(data_dir, company_id, session_id).join(format!("{uuid}.{ext}"))
}

pub async fn save_and_transcribe(
    or: &dyn LlmClient,
    model: &str,
    data_dir: &str,
    company_id: &str,
    session_id: &str,
    bytes: &[u8],
    ext: &str,
) -> Result<(String, String), AppError> {
    let dir = crate::recordings::session_dir(data_dir, company_id, session_id);
    crate::recordings::ensure_dir(&dir).await?;

    let id = uuid::Uuid::now_v7().to_string();
    let path = audio_path_for(data_dir, company_id, session_id, &id, ext);
    tokio::fs::write(&path, bytes).await?;

    let format = match ext.to_lowercase().as_str() {
        "webm" => "webm",
        "ogg" => "ogg",
        "mp3" => "mp3",
        "wav" => "wav",
        "m4a" | "mp4" => "mp4",
        _ => "webm",
    };

    let raw = or.stt(model, bytes, format).await?;
    let transcript = clean_transcript(&raw);

    Ok((path.to_string_lossy().into_owned(), transcript))
}

/// Strip leading/trailing quotes or fences a model sometimes wraps the
/// transcript in despite our "no preamble" instruction.
fn clean_transcript(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix("```").unwrap_or(s);
    let s = s.strip_suffix("```").unwrap_or(s);
    let s = s.trim();
    let s = s.strip_prefix('"').unwrap_or(s);
    let s = s.strip_suffix('"').unwrap_or(s);
    s.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_plain_text() {
        assert_eq!(clean_transcript("hello world"), "hello world");
    }

    #[test]
    fn case_triple_backticks_wrapping() {
        assert_eq!(clean_transcript("```hello```"), "hello");
    }

    #[test]
    fn case_double_quote_wrapping() {
        assert_eq!(clean_transcript("\"hello\""), "hello");
    }

    #[test]
    fn case_fence_then_quote() {
        // Order: trim → strip ``` prefix → strip ``` suffix → trim → strip "
        // prefix → strip " suffix → trim. So both layers come off.
        assert_eq!(clean_transcript("```\"hello\"```"), "hello");
    }

    #[test]
    fn case_only_leading_fence() {
        // strip_prefix succeeds, strip_suffix doesn't match — that's fine.
        assert_eq!(clean_transcript("```hello"), "hello");
    }

    #[test]
    fn case_whitespace_only() {
        assert_eq!(clean_transcript("   "), "");
    }
}

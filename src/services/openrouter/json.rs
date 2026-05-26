//! JSON post-processing for LLM output — fence unwrapping + safe parsing
//! with a structured-log fallback.

use serde::Deserialize;
use serde_json::Value;

use crate::error::AppError;

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

/// Try to parse model output as JSON (after unfencing). On failure:
/// 1. Log a `warn!` with a redacted preview of the raw response and the
///    parse error, so operators can diagnose without scraping logs for
///    full bodies.
/// 2. Return an `AppError::Upstream` whose **display** string omits the
///    upstream body — the body lives only in the warn log, never in the
///    chain bubbled back to the user / error template.
///
/// `op` is the short label (`"critique"`, `"summary"`, …) used in the log
/// event and the error display. Centralises the same five-line dance every
/// `*_lockin` / `summary` / `critique` service used to repeat.
pub fn parse_json_or_log<T: for<'de> Deserialize<'de>>(
    op: &'static str,
    raw: &str,
) -> Result<T, AppError> {
    parse_json::<T>(raw).map_err(|e| {
        tracing::warn!(
            op = %op,
            error = %e,
            raw_preview = %crate::services::redact::preview(raw, 240),
            "openrouter JSON parse failed",
        );
        AppError::Upstream(format!("{op} returned invalid JSON"))
    })
}

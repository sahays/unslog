//! Shared input sanitizer for free-form user text. Defense-in-depth at the
//! HTTP boundary — Askama auto-escape on output is still the load-bearing
//! XSS defense, this layer catches the *upstream* failures: oversized input,
//! null bytes, terminal-injection control chars, and CRLF mismatches between
//! the browser's `maxlength` (LF-counted) and the wire encoding (CRLF).
//!
//! Two flavors:
//!
//! * [`sanitize_short`] — single-line fields (names, roles, model ids).
//!   Newlines are rejected, not just normalized.
//! * [`sanitize_long`] — multi-line / body fields (chat, prompts, answers).
//!   CRLF → LF, tabs and newlines allowed, other controls rejected.
//!
//! Both trim outer whitespace, count Unicode scalar values (not bytes) for
//! the limit, and reject empty / oversized / null-bytes / other control
//! chars with a single `AppError::BadRequest` shape so handlers don't need
//! per-field error messages.

use crate::error::AppError;

/// Validate a single-line text field. Newlines and tabs are rejected — use
/// [`sanitize_long`] for body / multiline content.
pub fn sanitize_short(input: &str, max_chars: usize, field: &str) -> Result<String, AppError> {
    let trimmed = input.trim();
    enforce_non_empty(trimmed, field)?;
    enforce_max_chars(trimmed, max_chars, field)?;
    if trimmed
        .chars()
        .any(|c| c == '\0' || (c.is_control() && c != ' '))
    {
        return Err(AppError::BadRequest(format!(
            "{field} contains an invalid character"
        )));
    }
    Ok(trimmed.to_string())
}

/// Validate a multi-line text field. CRLF is normalized to LF so the
/// browser's `maxlength` (which counts LF as 1) matches the server-side
/// char count.
pub fn sanitize_long(input: &str, max_chars: usize, field: &str) -> Result<String, AppError> {
    let normalized = input.replace("\r\n", "\n").replace('\r', "\n");
    let trimmed = normalized.trim();
    enforce_non_empty(trimmed, field)?;
    enforce_max_chars(trimmed, max_chars, field)?;
    if trimmed
        .chars()
        .any(|c| c == '\0' || (c.is_control() && !matches!(c, '\n' | '\t')))
    {
        return Err(AppError::BadRequest(format!(
            "{field} contains an invalid character"
        )));
    }
    Ok(trimmed.to_string())
}

fn enforce_non_empty(trimmed: &str, field: &str) -> Result<(), AppError> {
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(format!("{field} is required")));
    }
    Ok(())
}

fn enforce_max_chars(trimmed: &str, max_chars: usize, field: &str) -> Result<(), AppError> {
    if trimmed.chars().count() > max_chars {
        return Err(AppError::BadRequest(format!(
            "{field} must be {max_chars} characters or fewer"
        )));
    }
    Ok(())
}

#[cfg(test)]
#[path = "text_validation_tests.rs"]
mod tests;

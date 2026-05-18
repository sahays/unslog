//! Logging discipline helpers. Centralizes the truncation + redaction
//! patterns so logging sites don't ad-hoc them.
//!
//! Use these any time a `tracing::*!` call wants to include user-typed text
//! or LLM output. Logs land in `data/logs/unslog.log` (daily-rotated, never
//! pruned by the app), so anything that lands there is effectively
//! permanent until the user manually wipes the log dir — treat it
//! accordingly.

/// Truncate to `n` Unicode scalar values, appending `…` when cut. Use this
/// for any user-typed field (names, titles, messages) before logging.
/// Default `n` for free-form fields elsewhere in the codebase is 200.
pub fn preview(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "…"
    }
}

/// Mask anything that looks like an email — keeps the local-part's first
/// character + the domain, replaces the rest with `…`. Good enough for "I
/// can still tell who this is roughly about" without leaving a usable
/// address in the logs. Not a security boundary — assume motivated humans
/// can still recover the value from context.
pub fn redact_emails(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut last_word_start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        let is_boundary = matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b',' | b';' | b'(' | b')' | b'<' | b'>' | b'"');
        if is_boundary {
            push_redacted(&mut out, &s[last_word_start..i]);
            out.push(b as char);
            last_word_start = i + 1;
        }
    }
    push_redacted(&mut out, &s[last_word_start..]);
    out
}

fn push_redacted(out: &mut String, word: &str) {
    if let Some(at) = word.find('@') {
        let local = &word[..at];
        let domain = &word[at..];
        if !local.is_empty() && domain.len() > 1 && domain[1..].contains('.') {
            out.push_str(&local[..1.min(local.len())]);
            out.push('…');
            out.push_str(domain);
            return;
        }
    }
    out.push_str(word);
}

#[cfg(test)]
#[path = "redact_tests.rs"]
mod tests;

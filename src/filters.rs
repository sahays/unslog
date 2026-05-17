//! Custom Askama filters.
//!
//! Askama looks up unknown filters at template-compile time, first in a local
//! `mod filters` (near the template struct), then in `crate::filters`. So
//! anything exposed from this module is available everywhere as
//! `{{ x|name }}`.

use pulldown_cmark::{html, Options, Parser};

/// "5m ago" / "3d ago" / "just now" — coarse relative-time formatting for
/// timestamps shown in the UI. Symmetric for future times ("5m from now").
/// Use via the `time_ago` Askama filter or directly in Rust.
pub fn time_ago(dt: &chrono::DateTime<chrono::Utc>) -> askama::Result<String> {
    Ok(format_relative_secs(
        chrono::Utc::now().signed_duration_since(*dt).num_seconds(),
    ))
}

fn format_relative_secs(secs: i64) -> String {
    const MIN: i64 = 60;
    const HOUR: i64 = 60 * MIN;
    const DAY: i64 = 24 * HOUR;
    const MONTH: i64 = 30 * DAY;
    const YEAR: i64 = 12 * MONTH;
    let (n, suffix) = if secs < 0 {
        (-secs, "from now")
    } else {
        (secs, "ago")
    };
    if n < 45 {
        "just now".into()
    } else if n < HOUR {
        format!("{}m {suffix}", (n + MIN / 2) / MIN)
    } else if n < DAY {
        format!("{}h {suffix}", n / HOUR)
    } else if n < MONTH {
        format!("{}d {suffix}", n / DAY)
    } else if n < YEAR {
        format!("{}mo {suffix}", n / MONTH)
    } else {
        format!("{}y {suffix}", n / YEAR)
    }
}

/// Render markdown to HTML, then run the result through an HTML sanitizer.
/// Use in place of the built-in `|markdown` filter when the input is
/// untrusted (e.g. LLM output) — raw HTML inside the markdown is treated as
/// literal text (`Options::ENABLE_HTML` is intentionally NOT set), and the
/// post-render output is ammonia-cleaned to strip anything that could carry
/// XSS.
///
/// Output is plain HTML; templates should follow with `|safe` to suppress
/// auto-escaping (we've already done both passes).
pub fn safe_markdown<S: AsRef<str>>(s: S) -> askama::Result<String> {
    let s = s.as_ref();
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    // Deliberately omit Options::ENABLE_HTML so raw <script> etc. inside
    // the markdown source is treated as text, not parsed.
    let parser = Parser::new_ext(s, opts);
    let mut rendered = String::with_capacity(s.len());
    html::push_html(&mut rendered, parser);
    Ok(ammonia::clean(&rendered))
}

#[cfg(test)]
#[path = "filters_tests.rs"]
mod tests;

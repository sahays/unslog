//! Custom Askama filters.
//!
//! Askama looks up unknown filters at template-compile time, first in a local
//! `mod filters` (near the template struct), then in `crate::filters`. So
//! anything exposed from this module is available everywhere as
//! `{{ x|name }}`.

use pulldown_cmark::{html, Options, Parser};

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

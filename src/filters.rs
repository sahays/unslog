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
mod tests {
    use super::*;

    fn render(s: &str) -> String {
        safe_markdown(s).expect("safe_markdown is infallible")
    }

    #[test]
    fn case_plain_markdown_renders() {
        let out = render("**bold**");
        assert!(out.contains("<strong>bold</strong>"), "got: {out}");
    }

    #[test]
    fn case_script_tag_stripped() {
        let out = render("<script>alert(1)</script>");
        assert!(!out.contains("<script"), "script tag leaked: {out}");
        assert!(!out.contains("alert(1)"), "script body leaked: {out}");
    }

    #[test]
    fn case_javascript_href_stripped() {
        let out = render("[click](javascript:alert(1))");
        assert!(!out.to_lowercase().contains("javascript:"), "got: {out}");
    }

    #[test]
    fn case_onerror_attr_stripped() {
        let out = render("<img src=x onerror=alert(1)>");
        assert!(!out.to_lowercase().contains("onerror"), "got: {out}");
    }

    #[test]
    fn case_data_uri_link_stripped() {
        let out = render("[x](data:text/html,<script>alert(1)</script>)");
        assert!(!out.to_lowercase().contains("data:text"), "got: {out}");
        assert!(!out.contains("<script"), "got: {out}");
    }

    #[test]
    fn case_code_fence_preserved() {
        let out = render("```\nlet x = 1;\n```");
        assert!(out.contains("<pre>"), "got: {out}");
        assert!(out.contains("<code"), "got: {out}");
        assert!(out.contains("let x = 1;"), "got: {out}");
    }

    #[test]
    fn case_table_renders() {
        let out = render("| a | b |\n|---|---|\n| 1 | 2 |\n");
        assert!(out.contains("<table>"), "ENABLE_TABLES not wired: {out}");
    }

    #[test]
    fn case_strikethrough_renders() {
        let out = render("~~gone~~");
        assert!(
            out.contains("<del>"),
            "ENABLE_STRIKETHROUGH not wired: {out}"
        );
    }
}

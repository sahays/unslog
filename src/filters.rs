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

/// Protocol allowlist for URLs that come from LLM output (research packet
/// sources, fetched_urls). Returns the input unchanged if it parses as
/// `http://` or `https://`; otherwise returns an empty string so the link
/// disappears in the rendered HTML.
///
/// Ammonia already strips `javascript:` etc. on `safe_markdown` output, but
/// these URLs are interpolated directly into `<a href="...">` outside of
/// that pipeline. This filter is the upstream defense.
pub fn safe_url<S: AsRef<str>>(s: S) -> askama::Result<String> {
    let trimmed = s.as_ref().trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Ok(trimmed.to_string())
    } else {
        Ok(String::new())
    }
}

/// Whitespace-split word count for free-form prose. Used by the spoken-
/// monologue view to surface "≈N words" next to each variant so the
/// candidate can eyeball delivery length at a glance.
pub fn word_count<S: AsRef<str>>(s: S) -> askama::Result<usize> {
    Ok(s.as_ref().split_whitespace().count())
}

/// Wrap a stored recording path as a `/recordings/...` URL. Lets templates
/// say `{{ p|recording_url }}` instead of reaching into the
/// `crate::recordings::to_url` Rust path from inside the template — keeps
/// view code from binding tightly to internal module layout.
pub fn recording_url<S: AsRef<str>>(p: S) -> askama::Result<String> {
    Ok(crate::recordings::to_url(p.as_ref()))
}

/// Resolve the in-flight request's signed-in user label. Ignores its input
/// — written as a filter so `base.html` can read the value via the standard
/// `{{ ""|current_user_label }}` pipeline without each template having to
/// thread the field through its own struct. Empty when no user is signed in.
pub fn current_user_label<S: AsRef<str>>(_unused: S) -> askama::Result<String> {
    Ok(crate::middleware::current_user_label())
}

/// Sibling of `current_user_label` for the in-flight CSRF token. Empty
/// outside a request scope or on the login page (where no session exists
/// yet — the login form mints its own token via the struct field).
pub fn csrf_token<S: AsRef<str>>(_unused: S) -> askama::Result<String> {
    Ok(crate::middleware::current_csrf_token())
}

/// True iff the signed-in user has `is_master = true`. Used by `base.html`
/// to conditionally render admin-only sidebar links. Defaults to false
/// outside a request scope, so /login (no user yet) hides those links.
pub fn current_user_is_master<S: AsRef<str>>(_unused: S) -> askama::Result<bool> {
    Ok(crate::middleware::current_user_is_master())
}

/// Resolve the in-flight request's last-24h usage snapshot as a
/// `(used, cap, pct)` triple. Templates read this via
/// `{% let (used, cap, pct) = ""|current_user_usage %}` then pass the
/// values into the `usage_bar` macro. Empty / unlimited (`cap == u64::MAX`)
/// for users who don't have a configured tier cap (master).
pub fn current_user_usage<S: AsRef<str>>(_unused: S) -> askama::Result<(u64, u64, u8)> {
    let w = crate::middleware::current_user_usage();
    Ok((w.used, w.cap, w.pct()))
}

#[cfg(test)]
#[path = "filters_tests.rs"]
mod tests;

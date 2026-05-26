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

#[test]
fn case_recording_url_strips_data_prefix() {
    let out = super::recording_url("data/recordings/co1/sess1/x.mp3").unwrap();
    assert_eq!(out, "/recordings/co1/sess1/x.mp3");
}

#[test]
fn case_recording_url_empty_when_no_recordings_segment() {
    assert_eq!(super::recording_url("/tmp/foo.mp3").unwrap(), "");
}

#[test]
fn case_time_ago_buckets() {
    use super::format_relative_secs as f;
    assert_eq!(f(0), "just now");
    assert_eq!(f(44), "just now");
    assert_eq!(f(45), "1m ago");
    assert_eq!(f(60), "1m ago");
    assert_eq!(f(89), "1m ago");
    assert_eq!(f(90), "2m ago");
    assert_eq!(f(59 * 60), "59m ago");
    assert_eq!(f(60 * 60), "1h ago");
    assert_eq!(f(23 * 3600), "23h ago");
    assert_eq!(f(24 * 3600), "1d ago");
    assert_eq!(f(29 * 86_400), "29d ago");
    assert_eq!(f(30 * 86_400), "1mo ago");
    assert_eq!(f(11 * 2_592_000), "11mo ago");
    assert_eq!(f(12 * 2_592_000), "1y ago");
    assert_eq!(f(-60), "1m from now");
}

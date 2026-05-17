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

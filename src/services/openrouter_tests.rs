use super::*;

// ── extract_fetched_urls ─────────────────────────────────────────────

fn ann(url: &str) -> Annotation {
    Annotation {
        url_citation: Some(UrlCitation { url: url.into() }),
    }
}

#[test]
fn case_extract_fetched_urls_empty() {
    assert!(extract_fetched_urls(&[]).is_empty());
}

#[test]
fn case_extract_fetched_urls_collects_in_order() {
    let anns = vec![ann("https://a.com"), ann("https://b.com")];
    assert_eq!(
        extract_fetched_urls(&anns),
        vec!["https://a.com".to_string(), "https://b.com".to_string()]
    );
}

#[test]
fn case_extract_fetched_urls_dedupes_preserving_first_position() {
    let anns = vec![
        ann("https://a.com"),
        ann("https://b.com"),
        ann("https://a.com"),
    ];
    assert_eq!(
        extract_fetched_urls(&anns),
        vec!["https://a.com".to_string(), "https://b.com".to_string()]
    );
}

#[test]
fn case_extract_fetched_urls_skips_annotation_without_citation() {
    let anns = vec![Annotation { url_citation: None }, ann("https://a.com")];
    assert_eq!(
        extract_fetched_urls(&anns),
        vec!["https://a.com".to_string()]
    );
}

#[test]
fn case_extract_fetched_urls_skips_empty_url() {
    let anns = vec![ann(""), ann("https://a.com")];
    assert_eq!(
        extract_fetched_urls(&anns),
        vec!["https://a.com".to_string()]
    );
}

// ── unwrap_fenced_json ───────────────────────────────────────────────

#[test]
fn case_plain_json_passthrough() {
    assert_eq!(unwrap_fenced_json(r#"{"a":1}"#), r#"{"a":1}"#);
}

#[test]
fn case_json_fence_stripped() {
    let input = "```json\n{\"a\":1}\n```";
    assert_eq!(unwrap_fenced_json(input), r#"{"a":1}"#);
}

#[test]
fn case_bare_fence_stripped() {
    let input = "```\n{\"a\":1}\n```";
    assert_eq!(unwrap_fenced_json(input), r#"{"a":1}"#);
}

#[test]
fn case_unclosed_fence_falls_through() {
    // No closing fence: the rfind("```") on `rest` finds the same prefix
    // we just stripped (when ```json), or finds nothing for bare ```.
    // Either way the function should not panic; if it falls through it
    // returns the trimmed input.
    let input = "```json\n{\"a\":1}";
    let out = unwrap_fenced_json(input);
    // Body still contains the JSON regardless of branch taken.
    assert!(out.contains(r#"{"a":1}"#), "got: {out}");
}

#[test]
fn case_leading_whitespace_trimmed() {
    let input = "   \n```json\n{\"a\":1}\n```\n   ";
    assert_eq!(unwrap_fenced_json(input), r#"{"a":1}"#);
}

#[test]
fn case_empty_input() {
    assert_eq!(unwrap_fenced_json(""), "");
}

#[test]
fn case_fence_with_inner_backticks() {
    // rfind matches the LAST ``` so a JSON string containing single
    // backticks is preserved.
    let input = "```json\n{\"a\":\"`x`\"}\n```";
    assert_eq!(unwrap_fenced_json(input), r#"{"a":"`x`"}"#);
}

// ── parse_json<T> ────────────────────────────────────────────────────

#[test]
fn case_parse_struct_from_fenced() {
    #[derive(Deserialize, Debug, PartialEq)]
    struct Toy {
        a: u32,
        b: String,
    }
    let raw = "```json\n{\"a\": 7, \"b\": \"hi\"}\n```";
    let out: Toy = parse_json(raw).expect("parse should succeed");
    assert_eq!(
        out,
        Toy {
            a: 7,
            b: "hi".to_string()
        }
    );
}

#[test]
fn case_parse_error_on_garbage() {
    #[derive(Deserialize)]
    struct Toy {
        #[allow(dead_code)]
        a: u32,
    }
    assert!(parse_json::<Toy>("not json").is_err());
}

// ── preview ──────────────────────────────────────────────────────────

#[test]
fn case_preview_short_passthrough() {
    assert_eq!(preview("hello", 10), "hello");
}

#[test]
fn case_preview_exact_length() {
    // length == n: no ellipsis
    assert_eq!(preview("hello", 5), "hello");
}

#[test]
fn case_preview_truncates_with_ellipsis() {
    assert_eq!(preview("hello world", 5), "hello…");
}

#[test]
fn case_preview_multibyte() {
    // Each emoji is a single codepoint; count is 4. Truncating to 2
    // must not slice mid-byte.
    let input = "🎉🎊🎈🎂";
    let out = preview(input, 2);
    assert_eq!(out, "🎉🎊…");
}

#[test]
fn case_preview_n_zero_nonempty() {
    assert_eq!(preview("hi", 0), "…");
}

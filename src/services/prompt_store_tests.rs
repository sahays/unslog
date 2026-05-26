use super::*;

#[test]
fn case_resolve_includes_inlines_known_marker() {
    let raw = "Header.\n\n{{include:_shared/three_bar_gate_short.md}}\n\nFooter.";
    let resolved = resolve_includes(raw);
    assert!(
        resolved.contains("Three-bar gate"),
        "snippet body not inlined: {resolved}",
    );
    assert!(
        !resolved.contains("{{include:"),
        "marker survived: {resolved}",
    );
    assert!(resolved.starts_with("Header."), "got: {resolved}");
    assert!(resolved.trim_end().ends_with("Footer."), "got: {resolved}");
}

#[test]
fn case_resolve_includes_leaves_unknown_markers_alone() {
    let raw = "{{include:_shared/nonexistent.md}}";
    assert_eq!(resolve_includes(raw), raw);
}

#[test]
fn case_resolve_includes_replaces_every_occurrence() {
    let raw = "{{include:_shared/action_vocab.md}} and again {{include:_shared/action_vocab.md}}";
    let resolved = resolve_includes(raw);
    assert!(!resolved.contains("{{include:"), "marker survived");
    // Both occurrences inlined: "Banned framings" appears once per snippet copy.
    let count = resolved.matches("Banned framings").count();
    assert_eq!(count, 2, "expected two inlined copies, got {count}");
}

#[test]
fn case_snippets_have_no_nested_includes() {
    // The resolver is single-pass; if a snippet body grew its own marker
    // we'd silently leak it into seeded prompts. Pin the contract.
    for (_, snippet) in SHARED_SNIPPETS {
        assert!(
            !snippet.contains("{{include:"),
            "snippet contains nested marker — resolver is single-pass",
        );
    }
}

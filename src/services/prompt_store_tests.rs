//! Tests defending system-prompt integrity for the LLM. Bugs here
//! silently corrupt every coach response: missing snippet content, stray
//! `{{include:...}}` markers reaching the model, or recursion smuggled
//! into seeded prompts.

use super::*;

/// LLM system-prompt integrity: a known include marker must inline the
/// snippet body in place, with no leftover marker text leaking through
/// to the model.
#[test]
fn resolve_includes_inlines_known_snippet_and_strips_marker() {
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

/// LLM system-prompt integrity: an unknown marker must be left verbatim
/// (not silently dropped) so a typo in the prompt file is visible to the
/// model author rather than producing a quietly different system prompt.
#[test]
fn resolve_includes_leaves_unknown_markers_verbatim_so_typos_are_visible() {
    let raw = "{{include:_shared/nonexistent.md}}";
    assert_eq!(resolve_includes(raw), raw);
}

/// LLM system-prompt integrity: every occurrence of a known marker must
/// be replaced — a partial replace would silently smuggle a marker into
/// the prompt body.
#[test]
fn resolve_includes_replaces_every_occurrence_no_marker_smuggled_through() {
    let raw = "{{include:_shared/action_vocab.md}} and again {{include:_shared/action_vocab.md}}";
    let resolved = resolve_includes(raw);
    assert!(!resolved.contains("{{include:"), "marker survived");
    let count = resolved.matches("Banned framings").count();
    assert_eq!(count, 2, "expected two inlined copies, got {count}");
}

/// LLM system-prompt integrity: the resolver is single-pass, so a
/// snippet body that contained its own `{{include:...}}` marker would
/// silently reach the model unresolved. Pin the contract.
#[test]
fn shared_snippets_contain_no_nested_include_markers_since_resolver_is_single_pass() {
    for (_, snippet) in SHARED_SNIPPETS {
        assert!(
            !snippet.contains("{{include:"),
            "snippet contains nested marker — resolver is single-pass",
        );
    }
}

//! Tests defending `<resume>` block integrity in LLM prompt context. A
//! malformed tag here lets a clever resume body break out of the
//! context wrapper and inject directives the coach would follow.

use super::*;
use std::sync::Arc;

/// LLM context integrity: the block must open and close with the exact
/// `<resume>` / `</resume>` tags the prompt template expects, and inner
/// whitespace must not bleed into surrounding prompt sections.
#[test]
fn wrap_emits_balanced_tags_and_trims_inner_whitespace() {
    let text = Arc::new("  Jane Q. Candidate\nSenior eng @ Acme\n".to_string());
    let out = wrap(&text);
    assert!(out.starts_with("<resume>\n"));
    assert!(out.ends_with("\n</resume>"));
    assert!(out.contains("Jane Q. Candidate\nSenior eng @ Acme"));
    assert!(!out.contains("  Jane"));
}

/// LLM context integrity: multi-line resume bodies must reach the model
/// with their line breaks intact — otherwise bullet/section structure
/// collapses into a single line and the model misreads role boundaries.
#[test]
fn wrap_preserves_inner_newlines_so_resume_structure_reaches_llm() {
    let text = Arc::new("line one\nline two\nline three".to_string());
    let out = wrap(&text);
    assert_eq!(out, "<resume>\nline one\nline two\nline three\n</resume>");
}

/// Handler availability: a whitespace-only resume must not panic; it
/// produces an empty-bodied tag pair (no useful context, no leak, no crash).
#[test]
fn wrap_does_not_panic_on_whitespace_only_resume_body() {
    let text = Arc::new("   \n  \n".to_string());
    let out = wrap(&text);
    assert_eq!(out, "<resume>\n\n</resume>");
}

/// LLM context integrity: empty blocks must be skipped so the prompt
/// doesn't grow stray blank separators that confuse section parsing.
#[test]
fn join_blocks_skips_empty_entries_so_prompt_has_no_stray_separators() {
    let out = join_blocks(&["alpha", "", "beta", ""]);
    assert_eq!(out, "alpha\n\nbeta");
}

//! Escape user-controlled values that land inside prompt-tag wrappers.
//!
//! Prompts use HTML-like delimiters (`<company_name>{value}</company_name>`,
//! `<pitch>...</pitch>`, etc.) to fence user input as *data* rather than
//! instructions. If a value contains a `<` or a closing tag, an attacker
//! (or a confused paste) can break out of the wrapper and inject directives
//! the model will follow. We HTML-entity-escape the four meta characters
//! before interpolation so the model only ever sees text inside the tags.
//!
//! Not a security boundary on its own — `llm_safety::check_output` still
//! scrubs leaked tags out of the response. This is the *input*-side half.

/// HTML-escape `s` for safe use as the body of a `<tag>...</tag>` block in
/// a prompt. Order matters: `&` must be replaced first, otherwise the
/// already-emitted `&amp;` from a subsequent rule would get double-escaped.
pub fn for_tag(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
#[path = "prompt_escape_tests.rs"]
mod tests;

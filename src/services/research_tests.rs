use super::*;

/// Feature integrity: the `:online` suffix is what triggers OpenRouter's
/// web-search routing. If this helper drops the suffix or double-appends
/// it, research silently runs without web search and the user sees stale
/// model knowledge with no error.
#[test]
fn ensure_online_suffix_appends_once_and_is_idempotent() {
    let added = ensure_online_suffix("google/gemini-2.5-pro");
    assert_eq!(added, "google/gemini-2.5-pro:online");
    assert_eq!(ensure_online_suffix(&added), added);
}

//! Unit tests for the write-pass FK helpers.

use super::super::id_map::{IdMap, Kind};
use super::rewrite_id_list;

#[test]
fn rewrite_id_list_remaps_known_company_ids() {
    let mut map = IdMap::new();
    let new_a = map.mint(Kind::Company, "old-a");
    let new_b = map.mint(Kind::Company, "old-b");
    let input = vec!["old-a".to_string(), "old-b".to_string()];
    let got = rewrite_id_list(&map, Kind::Company, &input);
    assert_eq!(got, vec![new_a, new_b]);
}

#[test]
fn rewrite_id_list_drops_unknown_ids_silently() {
    let mut map = IdMap::new();
    let kept = map.mint(Kind::Question, "live-id");
    let input = vec![
        "live-id".to_string(),
        "dangling-orphan".to_string(),
        "another-orphan".to_string(),
    ];
    let got = rewrite_id_list(&map, Kind::Question, &input);
    assert_eq!(got, vec![kept], "orphaned referents should be filtered out");
}

#[test]
fn rewrite_id_list_preserves_order() {
    let mut map = IdMap::new();
    let a = map.mint(Kind::Company, "a");
    let b = map.mint(Kind::Company, "b");
    let c = map.mint(Kind::Company, "c");
    let input = vec!["c".to_string(), "a".to_string(), "b".to_string()];
    let got = rewrite_id_list(&map, Kind::Company, &input);
    assert_eq!(got, vec![c, a, b]);
}

#[test]
fn rewrite_id_list_does_not_cross_kinds() {
    let mut map = IdMap::new();
    map.mint(Kind::Company, "shared-old-id");
    let input = vec!["shared-old-id".to_string()];
    // Look up under Question even though it was minted under Company:
    // strict per-Kind semantics mean we get nothing back.
    let got = rewrite_id_list(&map, Kind::Question, &input);
    assert!(got.is_empty(), "lookups must be Kind-scoped");
}

#[test]
fn rewrite_id_list_handles_empty_input() {
    let map = IdMap::new();
    let got = rewrite_id_list(&map, Kind::Company, &[]);
    assert!(got.is_empty());
}

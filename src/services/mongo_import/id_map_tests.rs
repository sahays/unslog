//! Unit tests for the id minter + FK map.

use super::*;
use std::collections::HashSet;

const ALL_KINDS: &[Kind] = &[
    Kind::Company,
    Kind::Question,
    Kind::Session,
    Kind::Evaluation,
    Kind::Summary,
    Kind::Story,
    Kind::StoryVersion,
    Kind::PitchVersion,
    Kind::JournalEntry,
    Kind::Asset,
    Kind::PromptVersion,
];

const ALPHABET_CHARS: &str = "abcdefghijklmnopqrstuvwxyz0123456789";

fn alphabet() -> HashSet<char> {
    ALPHABET_CHARS.chars().collect()
}

#[test]
fn mint_format_is_prefix_plus_six_lowercase_alphanum() {
    let alphabet = alphabet();
    for kind in ALL_KINDS {
        let id = mint(*kind);
        assert_eq!(id.len(), 9, "id {id} for kind {kind:?} should be 9 chars");
        assert!(
            id.starts_with(kind.prefix()),
            "id {id} for kind {kind:?} should start with {}",
            kind.prefix()
        );
        for c in id.chars().skip(3) {
            assert!(
                alphabet.contains(&c),
                "suffix char {c:?} in {id} not in alphabet"
            );
        }
    }
}

#[test]
fn id_map_mint_has_no_collisions_over_10k_samples_per_prefix() {
    // Bare `mint()` is probabilistic: 36^6 ≈ 2.18B with ~2% chance of a
    // birthday collision in 10k draws. The real call site is `IdMap::mint`,
    // which retries on collision and is the contract we depend on. Test
    // that contract here.
    for kind in ALL_KINDS {
        let mut map = IdMap::new();
        let mut seen: HashSet<String> = HashSet::with_capacity(10_000);
        for i in 0..10_000 {
            let old_id = format!("old-{i}");
            let new_id = map.mint(*kind, &old_id);
            assert!(
                seen.insert(new_id.clone()),
                "collision in 10k samples for {kind:?}: {new_id}"
            );
        }
    }
}

#[test]
fn id_map_mint_is_stable_for_same_old_id() {
    let mut map = IdMap::new();
    let first = map.mint(Kind::Company, "old-mongo-id");
    let second = map.mint(Kind::Company, "old-mongo-id");
    assert_eq!(first, second, "second mint should return cached id");
}

#[test]
fn id_map_mints_distinct_per_kind_for_same_old_id() {
    let mut map = IdMap::new();
    let c = map.mint(Kind::Company, "x");
    let q = map.mint(Kind::Question, "x");
    assert_ne!(c, q, "same old id under different kinds must mint distinct");
    assert!(c.starts_with("com"));
    assert!(q.starts_with("qst"));
}

#[test]
fn id_map_insert_fixed_overrides_minting() {
    let mut map = IdMap::new();
    map.insert_fixed(Kind::Company, "legacy-slug", "comabcdef");
    let got = map.get(Kind::Company, "legacy-slug");
    assert_eq!(got.as_deref(), Some("comabcdef"));
}

#[test]
fn id_map_get_returns_none_for_unknown() {
    let map = IdMap::new();
    assert!(map.get(Kind::Company, "missing").is_none());
}

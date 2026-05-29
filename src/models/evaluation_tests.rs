//! Tests defending the `Scores` serde round-trip (Mongo↔Rust) and the
//! `average()` divide-by-zero guard. The avg math itself is not tested
//! since a wrong score is a UX bug, not a corruption or availability bug.

use super::*;

fn scores(s: u8, r: u8, st: u8, p: u8, fit: Option<u8>) -> Scores {
    Scores {
        specificity: s,
        role_clarity: r,
        star_plus_structure: st,
        pitfalls_avoided: p,
        company_fit: fit,
    }
}

/// Handler availability: if `average` ever divides by zero the request crashes.
#[test]
fn average_does_not_panic_on_all_zero_scores() {
    assert_eq!(scores(0, 0, 0, 0, None).average(), 0.0);
    assert_eq!(scores(0, 0, 0, 0, Some(0)).average(), 0.0);
}

// ── deserialize_optional_u8: db↔rust round-trip integrity ────────────

#[test]
fn company_fit_null_preserved_through_db_serde() {
    let json = r#"{"specificity":1,"role_clarity":1,"star_plus_structure":1,"pitfalls_avoided":1,"company_fit":null}"#;
    let s: Scores = serde_json::from_str(json).expect("parse");
    assert_eq!(s.company_fit, None);
}

#[test]
fn company_fit_missing_preserved_through_db_serde() {
    let json = r#"{"specificity":1,"role_clarity":1,"star_plus_structure":1,"pitfalls_avoided":1}"#;
    let s: Scores = serde_json::from_str(json).expect("parse");
    assert_eq!(s.company_fit, None);
}

#[test]
fn company_fit_number_preserved_through_db_serde() {
    let json = r#"{"specificity":1,"role_clarity":1,"star_plus_structure":1,"pitfalls_avoided":1,"company_fit":3}"#;
    let s: Scores = serde_json::from_str(json).expect("parse");
    assert_eq!(s.company_fit, Some(3));
}

/// Pinned behavior: literal 0 deserializes to Some(0), NOT None — the
/// doc-comment on `deserialize_optional_u8` suggests otherwise, but the
/// impl just delegates to Option::<u8>::deserialize. Pinned so a future
/// "fix" to match the doc-comment doesn't silently change displayed scores.
#[test]
fn company_fit_zero_serdes_as_some_zero_not_none() {
    let json = r#"{"specificity":1,"role_clarity":1,"star_plus_structure":1,"pitfalls_avoided":1,"company_fit":0}"#;
    let s: Scores = serde_json::from_str(json).expect("parse");
    assert_eq!(s.company_fit, Some(0));
}

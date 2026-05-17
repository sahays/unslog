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

// ── Scores::average ──────────────────────────────────────────────────

#[test]
fn case_avg_without_company_fit() {
    // (1+2+3+4)/4 = 2.5
    assert_eq!(scores(1, 2, 3, 4, None).average(), 2.5);
}

#[test]
fn case_avg_with_company_fit() {
    // (2+2+2+2+5)/5 = 2.6
    assert_eq!(scores(2, 2, 2, 2, Some(5)).average(), 2.6);
}

#[test]
fn case_avg_zero_no_panic() {
    // All zeros — must not divide by zero (n is 4 or 5, never 0).
    assert_eq!(scores(0, 0, 0, 0, None).average(), 0.0);
    assert_eq!(scores(0, 0, 0, 0, Some(0)).average(), 0.0);
}

#[test]
fn case_avg_max() {
    assert_eq!(scores(5, 5, 5, 5, Some(5)).average(), 5.0);
}

// ── deserialize_optional_u8 (via Scores serde round-trip) ────────────

#[test]
fn case_company_fit_null() {
    let json = r#"{"specificity":1,"role_clarity":1,"star_plus_structure":1,"pitfalls_avoided":1,"company_fit":null}"#;
    let s: Scores = serde_json::from_str(json).expect("parse");
    assert_eq!(s.company_fit, None);
}

#[test]
fn case_company_fit_missing() {
    let json = r#"{"specificity":1,"role_clarity":1,"star_plus_structure":1,"pitfalls_avoided":1}"#;
    let s: Scores = serde_json::from_str(json).expect("parse");
    assert_eq!(s.company_fit, None);
}

#[test]
fn case_company_fit_number() {
    let json = r#"{"specificity":1,"role_clarity":1,"star_plus_structure":1,"pitfalls_avoided":1,"company_fit":3}"#;
    let s: Scores = serde_json::from_str(json).expect("parse");
    assert_eq!(s.company_fit, Some(3));
}

#[test]
fn case_company_fit_zero_is_some_zero() {
    // Pin current behavior: a literal 0 deserializes to Some(0), NOT
    // None. Doc-comment on deserialize_optional_u8 claims otherwise; the
    // implementation just delegates to Option::<u8>::deserialize so this
    // is what actually happens.
    let json = r#"{"specificity":1,"role_clarity":1,"star_plus_structure":1,"pitfalls_avoided":1,"company_fit":0}"#;
    let s: Scores = serde_json::from_str(json).expect("parse");
    assert_eq!(s.company_fit, Some(0));
}

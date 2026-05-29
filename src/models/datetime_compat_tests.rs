//! Tests defending the Mongo↔Rust datetime serde shim. Silent breakage
//! here corrupts every persisted timestamp; the `required` invalid-input
//! path also guards against silently accepting malformed data from disk
//! or wire.

use super::*;

#[derive(Debug, Serialize, Deserialize)]
struct Req {
    #[serde(with = "super::required")]
    ts: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Opt {
    #[serde(with = "super::optional", default)]
    ts: Option<DateTime<Utc>>,
}

/// DB integrity: RFC3339 strings round-trip into `DateTime<Utc>` without
/// timezone drift or precision loss.
#[test]
fn required_round_trips_rfc3339_through_serde_without_drift() {
    let r: Req = serde_json::from_str(r#"{"ts":"2024-01-02T03:04:05Z"}"#).expect("parse");
    assert_eq!(r.ts.to_rfc3339(), "2024-01-02T03:04:05+00:00");
}

/// Data-integrity boundary: malformed timestamps must error, not silently
/// substitute a default — otherwise corrupted data slips through.
#[test]
fn required_errors_on_invalid_string_rather_than_substituting_default() {
    let res: Result<Req, _> = serde_json::from_str(r#"{"ts":"not-a-date"}"#);
    assert!(res.is_err(), "expected error, got {:?}", res);
}

/// DB integrity: explicit nulls must deserialize as None — otherwise the
/// "no timestamp set" state collapses into a sentinel epoch value.
#[test]
fn optional_null_preserves_none_through_serde() {
    let o: Opt = serde_json::from_str(r#"{"ts":null}"#).expect("parse");
    assert!(o.ts.is_none());
}

/// DB integrity: Some/None round-trip on serialize so the persisted shape
/// matches what was read in.
#[test]
fn optional_round_trips_some_and_none_on_serialize() {
    let dt = DateTime::parse_from_rfc3339("2024-01-02T03:04:05Z")
        .unwrap()
        .with_timezone(&Utc);
    let some = Opt { ts: Some(dt) };
    let none = Opt { ts: None };
    let s_some = serde_json::to_string(&some).expect("serialize some");
    let s_none = serde_json::to_string(&none).expect("serialize none");
    assert!(s_some.contains("2024-01-02T03:04:05"), "got: {s_some}");
    assert!(s_none.contains("null"), "got: {s_none}");
}

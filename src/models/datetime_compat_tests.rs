use super::*;

// Wrappers that exercise the `required` and `optional` modules through
// the same `#[serde(with = ...)]` plumbing the real models use.
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

#[test]
fn case_required_rfc3339_string() {
    let r: Req = serde_json::from_str(r#"{"ts":"2024-01-02T03:04:05Z"}"#).expect("parse");
    assert_eq!(r.ts.to_rfc3339(), "2024-01-02T03:04:05+00:00");
}

#[test]
fn case_required_invalid_string_errors() {
    let res: Result<Req, _> = serde_json::from_str(r#"{"ts":"not-a-date"}"#);
    assert!(res.is_err(), "expected error, got {:?}", res);
}

#[test]
fn case_required_serializes_rfc3339() {
    let dt = DateTime::parse_from_rfc3339("2024-01-02T03:04:05Z")
        .unwrap()
        .with_timezone(&Utc);
    let r = Req { ts: dt };
    let out = serde_json::to_string(&r).expect("serialize");
    assert!(out.contains("2024-01-02T03:04:05"), "got: {out}");
}

#[test]
fn case_optional_null() {
    let o: Opt = serde_json::from_str(r#"{"ts":null}"#).expect("parse");
    assert!(o.ts.is_none());
}

#[test]
fn case_optional_some_string() {
    let o: Opt = serde_json::from_str(r#"{"ts":"2024-01-02T03:04:05Z"}"#).expect("parse");
    assert!(o.ts.is_some());
    assert_eq!(o.ts.unwrap().to_rfc3339(), "2024-01-02T03:04:05+00:00");
}

#[test]
fn case_optional_serializes_some_and_none() {
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

//! Tolerant chrono DateTime serde for round-tripping through MongoDB.
//!
//! Reads accept either an RFC 3339 string (chrono's default serde format)
//! OR a native BSON DateTime — early rows in some collections were written
//! through code paths that produced native BSON dates, and we need both
//! shapes to deserialize cleanly.
//!
//! Writes always emit an RFC 3339 string so the format is stable going
//! forward; once a row is re-saved it round-trips cleanly.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Deserialize)]
#[serde(untagged)]
enum AnyDateTime {
    Bson(bson::DateTime),
    Rfc3339(String),
}

impl AnyDateTime {
    fn into_chrono(self) -> Result<DateTime<Utc>, String> {
        match self {
            AnyDateTime::Bson(b) => Ok(b.to_chrono()),
            AnyDateTime::Rfc3339(s) => DateTime::parse_from_rfc3339(&s)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| format!("invalid rfc3339: {e}")),
        }
    }
}

pub mod required {
    use super::*;

    pub fn serialize<S: Serializer>(dt: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
        dt.to_rfc3339().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Utc>, D::Error> {
        AnyDateTime::deserialize(d)?
            .into_chrono()
            .map_err(serde::de::Error::custom)
    }
}

pub mod optional {
    use super::*;

    pub fn serialize<S: Serializer>(dt: &Option<DateTime<Utc>>, s: S) -> Result<S::Ok, S::Error> {
        match dt {
            Some(d) => d.to_rfc3339().serialize(s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<DateTime<Utc>>, D::Error> {
        let v = Option::<AnyDateTime>::deserialize(d)?;
        v.map(|x| x.into_chrono().map_err(serde::de::Error::custom))
            .transpose()
    }
}

#[cfg(test)]
mod tests {
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
}

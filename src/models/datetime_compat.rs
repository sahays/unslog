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

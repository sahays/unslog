//! Prefix-aware id minter + Mongo → Postgres FK map.
//!
//! IDs are 9 chars: `{3-letter prefix}{6 lowercase alphanum}`. Catalog
//! tables (categories, pitches) keep their slug ids; the master user is
//! the literal `usrmaster`. Everything else gets a fresh prefixed id.

use rand::rngs::OsRng;
use rand::RngCore;
use std::collections::HashMap;

/// Reserved literal id for the single master user.
pub const MASTER_USER_ID: &str = "usrmaster";

/// Singleton id mandated by the Postgres `settings` table check constraint.
pub const SETTINGS_SINGLETON_ID: &str = "singleton";

/// Lowercase-alphanum alphabet used for the 6-char random suffix.
const ALPHABET: &[u8; 36] = b"abcdefghijklmnopqrstuvwxyz0123456789";

/// Domain object kinds that get prefixed ids. Catalog / singleton kinds
/// are intentionally absent — those use slugs / fixed literals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Company,
    Question,
    Session,
    Evaluation,
    Summary,
    Story,
    StoryVersion,
    PitchVersion,
    JournalEntry,
    Asset,
    PromptVersion,
}

impl Kind {
    pub fn prefix(self) -> &'static str {
        match self {
            Kind::Company => "com",
            Kind::Question => "qst",
            Kind::Session => "ses",
            Kind::Evaluation => "eva",
            Kind::Summary => "sum",
            Kind::Story => "sto",
            Kind::StoryVersion => "stv",
            Kind::PitchVersion => "piv",
            Kind::JournalEntry => "jnl",
            Kind::Asset => "ast",
            Kind::PromptVersion => "prv",
        }
    }
}

/// Mint a fresh `{prefix}{6}` id using OS CSPRNG. The 9-char total leaves
/// 36^6 ≈ 2.1B distinct values per prefix — collision under 10k samples is
/// effectively zero, but [`IdMap::mint`] still retries on the unlikely
/// duplicate.
pub fn mint(kind: Kind) -> String {
    let mut suffix = [0u8; 6];
    let mut raw = [0u8; 6];
    OsRng.fill_bytes(&mut raw);
    for (i, byte) in raw.iter().enumerate() {
        suffix[i] = ALPHABET[(*byte as usize) % ALPHABET.len()];
    }
    let mut out = String::with_capacity(9);
    out.push_str(kind.prefix());
    // SAFETY: `suffix` only ever holds bytes from `ALPHABET`, which is ASCII.
    out.push_str(std::str::from_utf8(&suffix).expect("alphabet is ascii"));
    out
}

/// Mongo→Postgres id rewrite table, keyed by `(kind, old_id)`. Built in
/// the read pass; consulted in the write pass so every FK is replaced
/// atomically without a second DB round-trip.
#[derive(Default)]
pub struct IdMap {
    inner: HashMap<(Kind, String), String>,
    minted_suffixes: HashMap<Kind, std::collections::HashSet<String>>,
}

impl IdMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint a new id for `(kind, old_id)` and remember the mapping. Retries
    /// once on the vanishingly rare collision against already-minted ids of
    /// the same kind — a correctness backstop, not a hot path.
    pub fn mint(&mut self, kind: Kind, old_id: &str) -> String {
        if let Some(existing) = self.inner.get(&(kind, old_id.to_string())) {
            return existing.clone();
        }
        let seen = self.minted_suffixes.entry(kind).or_default();
        let new_id = loop {
            let candidate = mint(kind);
            if seen.insert(candidate.clone()) {
                break candidate;
            }
        };
        self.inner
            .insert((kind, old_id.to_string()), new_id.clone());
        new_id
    }

    /// Record a fixed mapping (e.g. catalog slug-to-slug for clarity, or
    /// master-user override). Returns the value stored.
    pub fn insert_fixed(&mut self, kind: Kind, old_id: &str, new_id: &str) -> String {
        self.inner
            .insert((kind, old_id.to_string()), new_id.to_string());
        new_id.to_string()
    }

    /// Lookup without minting. Returns `None` for unknown ids so callers
    /// can decide between "leave as-is" (catalog slugs) and "skip the row".
    pub fn get(&self, kind: Kind, old_id: &str) -> Option<String> {
        self.inner.get(&(kind, old_id.to_string())).cloned()
    }
}

#[cfg(test)]
#[path = "id_map_tests.rs"]
mod tests;

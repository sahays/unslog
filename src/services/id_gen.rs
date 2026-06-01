//! Shared, prefix-aware id minter used by every store that owns a
//! prefixed-id Postgres row. Promoted out of `mongo_import::id_map` so
//! production code paths (journal_store, asset_store, prompt_store, …)
//! and the one-shot importer share one minter — see `CLAUDE.md`'s DRY
//! guidance.
//!
//! IDs are 9 chars: `{3-letter prefix}{6 lowercase alphanum}`. The
//! Postgres CHECK constraints in `migrations/0001_initial.sql` pin the
//! exact same regex (`^<prefix>[a-z0-9]{6}$`) so any drift between
//! `Kind::prefix` and the schema is caught at insert time.

use rand::rngs::OsRng;
use rand::RngCore;

/// Lowercase-alphanum alphabet used for the 6-char random suffix. 36^6 ≈
/// 2.1B distinct values per prefix — collision probability under 10k
/// samples is effectively zero. Callers that need an idempotent retry
/// loop wrap [`new`] themselves.
const ALPHABET: &[u8; 36] = b"abcdefghijklmnopqrstuvwxyz0123456789";

/// Domain object kinds that get prefixed ids. Catalog / singleton kinds
/// (categories, pitches, settings, master user) are intentionally absent —
/// those use slugs / fixed literals.
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

/// Mint a fresh `{prefix}{6}` id using the OS CSPRNG. Bytes are uniformly
/// folded into [`ALPHABET`] indices via modulo (acceptable bias for
/// 256 mod 36 — non-uniformity is < 0.4 % per char, well inside the
/// collision-probability budget).
pub fn new(kind: Kind) -> String {
    let mut raw = [0u8; 6];
    OsRng.fill_bytes(&mut raw);
    let mut suffix = [0u8; 6];
    for (i, byte) in raw.iter().enumerate() {
        suffix[i] = ALPHABET[(*byte as usize) % ALPHABET.len()];
    }
    let mut out = String::with_capacity(9);
    out.push_str(kind.prefix());
    // SAFETY: `suffix` only ever holds bytes from `ALPHABET`, which is ASCII.
    out.push_str(std::str::from_utf8(&suffix).expect("alphabet is ascii"));
    out
}

/// True iff `id` matches the `{prefix}{6 lowercase alphanum}` shape for
/// `kind`. Mirrors the per-table Postgres CHECK so callers (importer,
/// tests) can validate without a DB round-trip.
pub fn is_valid(kind: Kind, id: &str) -> bool {
    let prefix = kind.prefix();
    id.len() == 9
        && id.starts_with(prefix)
        && id[3..]
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

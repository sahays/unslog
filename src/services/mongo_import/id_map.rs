//! Mongo → Postgres FK rewrite table.
//!
//! The minter itself lives in [`crate::services::id_gen`] — see that module
//! for the `{prefix}{6}` shape contract and the [`Kind`] enum. This file
//! only owns the bin-specific `IdMap` that records old→new mappings across
//! the read pass for the write pass to consult.
//!
//! Catalog tables (categories, pitches) keep their slug ids; the master
//! user is the literal `usrmaster`. Everything else gets a fresh prefixed
//! id minted through [`IdMap::mint`].

use std::collections::{HashMap, HashSet};

// Re-export the shared minter and Kind enum so existing callers
// (`mongo_import::*`, `models::prompt::PromptVersion::new`) continue to
// resolve `Kind` and `mint` through this module.
pub use crate::services::id_gen::Kind;

/// Reserved literal id for the single master user.
pub const MASTER_USER_ID: &str = "usrmaster";

/// Singleton id mandated by the Postgres `settings` table check constraint.
pub const SETTINGS_SINGLETON_ID: &str = "singleton";

/// Free-function wrapper kept for back-compat with the existing call sites
/// (`models::prompt::PromptVersion::new`, the bin minting helpers). New
/// code should call [`crate::services::id_gen::new`] directly.
pub fn mint(kind: Kind) -> String {
    crate::services::id_gen::new(kind)
}

/// Mongo→Postgres id rewrite table, keyed by `(kind, old_id)`. Built in
/// the read pass; consulted in the write pass so every FK is replaced
/// atomically without a second DB round-trip.
#[derive(Default)]
pub struct IdMap {
    inner: HashMap<(Kind, String), String>,
    minted_suffixes: HashMap<Kind, HashSet<String>>,
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
            let candidate = crate::services::id_gen::new(kind);
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

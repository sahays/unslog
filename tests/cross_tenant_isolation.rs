//! Cross-tenant isolation suite.
//!
//! Each `#[sqlx::test]` spins up a fresh per-test database, runs every
//! migration in `./migrations`, then drives the production store functions
//! against it. The contract under test: every owner-scoped store keeps
//! user A's rows invisible/immutable/undeletable to user B, even when B
//! knows A's row ids.
//!
//! Postgres must be reachable on the `DATABASE_URL` configured in `.env`
//! (see `scripts/check-deps.sh`). `#[sqlx::test]` creates a per-test
//! database on that server, so no manual cleanup is required.
//!
//! Suite layout — one module per owner-scoped store. The shared helper
//! lives in `common`; per-store helpers stay in their own module so each
//! file stays under the 300 LOC budget.

mod cross_tenant {
    pub mod assets;
    pub mod common;
    pub mod companies;
    pub mod evaluations;
    pub mod journal;
    pub mod pitches;
    pub mod questions;
    pub mod sessions;
    pub mod sharing;
    pub mod stories;
    pub mod summaries;
}

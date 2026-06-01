//! Metering: per-request cost accounting + tier-cap enforcement.
//!
//! Three pieces, kept small:
//! * `weights` — pure (method, path) → (kind, units) lookup table.
//! * `cap` — `MeteringDeps` trait + the cap-check policy. Cap window is
//!   rolling 24h; master tier is always exempt.
//! * `log` — fire-and-forget `request_log` append. Errors are warn-logged
//!   and dropped; metering is forensic, never gating.
//!
//! The middleware in `crate::middleware::metering` composes these.

pub mod cap;
pub mod log;
pub mod weights;

pub use cap::{check_or_block, current_window, AppendRow, MeteringDeps};
pub use weights::{metered_templates, weight_for};

//! Fire-and-forget append for the `request_log` table.
//!
//! Failures here MUST never surface to the caller — metering is forensic,
//! not gating. Errors are logged at `warn` so we can spot them in the
//! daily-rotating log, then dropped.

use std::sync::Arc;

use crate::models::RequestKind;

use super::cap::{AppendRow, MeteringDeps};

/// One row's worth of audit data — passed to [`spawn`] as a single value
/// so we don't blow the 7-arg clippy budget.
pub struct LogEntry {
    pub user_id: String,
    pub method: String,
    pub path: String,
    pub status: i16,
    pub duration_ms: i32,
    pub kind: RequestKind,
    pub cost_units: u32,
}

/// Spawn the append onto the runtime. The caller returns immediately;
/// the row eventually lands or the warn-line is emitted.
pub fn spawn(deps: Arc<dyn MeteringDeps>, entry: LogEntry) {
    tokio::spawn(async move {
        let log_uid = entry.user_id.clone();
        let log_method = entry.method.clone();
        let log_path = entry.path.clone();
        let row = AppendRow {
            user_id: entry.user_id,
            method: entry.method,
            path: entry.path,
            status: entry.status,
            duration_ms: entry.duration_ms,
            kind: entry.kind.as_str().to_string(),
            cost_units: entry.cost_units,
        };
        if let Err(e) = deps.append(row).await {
            tracing::warn!(
                event = "meter.log_failed",
                user_id = %log_uid,
                method = %log_method,
                path = %log_path,
                error = %e,
                "could not append request_log row",
            );
        }
    });
}

//! Request-cost metering middleware.
//!
//! Layered AFTER `auth_middleware` so `CurrentUser` is always present in
//! extensions, and BEFORE `csrf_verify_middleware` so cap rejections are
//! emitted independently of CSRF outcome (CSRF rejections still log
//! fire-and-forget after they happen, just with the post-CSRF status).
//!
//! Flow:
//! 1. Lookup `(kind, weight)` for `(method, path)`.
//! 2. If `weight > 0`, cap-check. Over-cap → return `RateLimited` (no
//!    handler call, no log row).
//! 3. Publish the snapshotted `UsageWindow` into the task-local so the
//!    sidebar bar can read it via the `current_user_usage` filter.
//! 4. Run the handler.
//! 5. Spawn a fire-and-forget log append. Failed requests (4xx/5xx) get
//!    `cost_units = 0` so users aren't billed for handler failures.

use std::time::Instant;

use axum::extract::{Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response;

use crate::error::AppError;
use crate::models::UsageWindow;
use crate::services::auth::CurrentUser;
use crate::services::metering::{self, weight_for};
use crate::startup::AppState;

tokio::task_local! {
    /// Snapshot of the current user's last-24h usage. Read by the
    /// `current_user_usage` Askama filter to render the sidebar bar
    /// without each template having to thread the values through its
    /// own struct.
    pub static CURRENT_USER_USAGE: UsageWindow;
}

/// Best-effort lookup of the current request's usage window. Returns an
/// empty `(used=0, cap=u64::MAX)` outside a request scope so the bar
/// renders as "no quota / unlimited" on pre-auth paths (the template
/// inspects `cap == u64::MAX` and hides the bar anyway).
pub fn current_user_usage() -> UsageWindow {
    CURRENT_USER_USAGE.try_with(|w| *w).unwrap_or(UsageWindow {
        used: 0,
        cap: u64::MAX,
    })
}

/// Routes that bypass metering entirely (no cap-check, no log row).
fn is_metering_exempt(path: &str) -> bool {
    matches!(path, "/login" | "/logout" | "/health")
        || path.starts_with("/static/")
        || path.starts_with("/recordings/")
}

/// Axum 0.7 middleware closure. Mounted globally via `from_fn_with_state`
/// after `auth_middleware`.
pub async fn metering_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let path = req.uri().path().to_string();
    if is_metering_exempt(&path) {
        return Ok(next.run(req).await);
    }
    let Some(current_user) = req.extensions().get::<CurrentUser>().cloned() else {
        // No user on a protected path — auth middleware will have
        // redirected already; this branch is a defensive no-op.
        return Ok(next.run(req).await);
    };
    let method = req.method().clone();
    let (kind, weight) = weight_for(&method, &path);
    let snapshot = cap_check_and_snapshot(&state, &current_user, weight).await?;
    let started = Instant::now();
    let response = CURRENT_USER_USAGE.scope(snapshot, next.run(req)).await;
    spawn_log(LogContext {
        state: &state,
        user_id: &current_user.id,
        method: &method,
        path: &path,
        response: &response,
        started,
        kind,
        weight,
    });
    Ok(response)
}

/// Run the cap-check when the request has a non-zero weight; otherwise
/// just snapshot the current window so the bar still gets populated.
async fn cap_check_and_snapshot(
    state: &AppState,
    user: &CurrentUser,
    weight: u32,
) -> Result<UsageWindow, AppError> {
    let deps = state.metering_deps.as_ref();
    if weight == 0 {
        return Ok(metering::current_window(deps, user).await);
    }
    metering::check_or_block(deps, user, weight).await
}

/// Inputs bundled into one struct so `spawn_log` stays under the 7-arg
/// clippy budget. Borrowed; copied into the `LogEntry` on the spawn path.
struct LogContext<'a> {
    state: &'a AppState,
    user_id: &'a str,
    method: &'a Method,
    path: &'a str,
    response: &'a Response,
    started: Instant,
    kind: crate::models::RequestKind,
    weight: u32,
}

/// Fire-and-forget log append. Charged units are 0 on failed responses so
/// the user isn't billed for a handler error.
fn spawn_log(ctx: LogContext<'_>) {
    let duration_ms = ctx.started.elapsed().as_millis().min(i32::MAX as u128) as i32;
    let status = ctx.response.status().as_u16() as i16;
    let billed = if ctx.response.status().is_success() || ctx.response.status().is_redirection() {
        ctx.weight
    } else {
        0
    };
    metering::log::spawn(
        ctx.state.metering_deps.clone(),
        metering::log::LogEntry {
            user_id: ctx.user_id.to_string(),
            method: ctx.method.to_string(),
            path: ctx.path.to_string(),
            status,
            duration_ms,
            kind: ctx.kind,
            cost_units: billed,
        },
    );
}

#[cfg(test)]
#[path = "metering_tests.rs"]
mod tests;

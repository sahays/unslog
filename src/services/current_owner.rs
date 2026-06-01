//! Temporary scaffold for the per-user data-ownership column.
//!
//! Phase 1 introduces a real `User` model and a `CurrentUser` extractor;
//! until then every per-user query and insert is hard-pinned to the
//! `usrmaster` user that migration 0002 seeds. When Phase 1 lands, search
//! for `TEMP_OWNER_ID` and replace each call site with the request-bound
//! `current_user.id` — the only place this constant is read should be the
//! routes/services that filter/insert on `owner_id`.

// TODO(phase-1): replace every call site with `current_user.id` and
// delete this constant + module.
pub const TEMP_OWNER_ID: &str = "usrmaster";

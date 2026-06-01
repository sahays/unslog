//! Process-local cache of current prompt bodies keyed by `prompt_name`.
//!
//! Every chat / lockin / critique / summary call reads the current body
//! for one of ~10 prompts. The underlying row only changes when the user
//! saves a new version on the Agents page. We cache `Arc<String>` so
//! cache hits don't clone the body; callers borrow into the prompt
//! template directly. Invalidate from `prompt_store::save_version` callers.
//!
//! Mirrors `BookCache` / `ModelsCache` shape: `Arc<RwLock<…>>`,
//! explicit invalidation, no TTL.

use std::collections::HashMap;
use std::sync::Arc;

use sqlx::PgPool;
use tokio::sync::RwLock;

use crate::error::AppError;
use crate::services::prompt_store;

#[derive(Clone, Default)]
pub struct PromptCache {
    inner: Arc<RwLock<HashMap<String, Arc<String>>>>,
}

impl PromptCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the current body for `name`, loading + caching on first
    /// read. Returns `Arc<String>` so the body can be cloned cheaply
    /// across many concurrent chat sessions.
    pub async fn get(&self, pool: &PgPool, name: &str) -> Result<Arc<String>, AppError> {
        {
            let guard = self.inner.read().await;
            if let Some(body) = guard.get(name) {
                return Ok(body.clone());
            }
        }
        let fresh = Arc::new(prompt_store::get_current_body(pool, name).await?);
        let mut guard = self.inner.write().await;
        guard.insert(name.to_string(), fresh.clone());
        Ok(fresh)
    }

    /// Like [`get`], but appends the output schema (if any) — same
    /// contract as [`prompt_store::get_current_body_with_schema`]. The
    /// cache stores the raw body and we append the schema per-call; the
    /// schema is `&'static str` so the append cost is negligible and we
    /// don't have to cache two variants per name.
    pub async fn get_with_schema(&self, pool: &PgPool, name: &str) -> Result<String, AppError> {
        let body = self.get(pool, name).await?;
        Ok(prompt_store::with_schema(name, body.as_ref().clone()))
    }

    /// Drop the cached body for `name`. Called from `save_version`
    /// callers and the restore handler so the next read sees the new
    /// current version.
    pub async fn invalidate(&self, name: &str) {
        let mut guard = self.inner.write().await;
        guard.remove(name);
    }
}

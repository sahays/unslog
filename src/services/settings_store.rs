//! Settings singleton: load-or-seed-defaults, save.
//!
//! Backed by Postgres `settings` table. The `id` column is pinned to
//! `Settings::SINGLETON_ID` by a CHECK constraint, so even a buggy caller
//! that tries to insert a second row will be rejected at the database
//! boundary — not just by application code.

use std::sync::Arc;

use sqlx::PgPool;
use tokio::sync::RwLock;

use crate::error::AppError;
use crate::models::Settings;
use crate::services::openrouter;

/// Read the singleton, seeding compile-time defaults on first read.
pub async fn load(pool: &PgPool) -> Result<Settings, AppError> {
    if let Some(s) = fetch(pool).await? {
        return Ok(s);
    }
    // First-boot path: insert defaults and re-read. `ON CONFLICT DO NOTHING`
    // makes the insert race-safe — a parallel caller racing us still gets
    // exactly one row total and a clean read back out.
    seed_singleton(pool).await?;
    fetch(pool)
        .await?
        .ok_or_else(|| AppError::Other(anyhow::anyhow!("settings singleton missing after seed")))
}

async fn fetch(pool: &PgPool) -> Result<Option<Settings>, AppError> {
    let row = sqlx::query_as!(
        Settings,
        r#"
        SELECT id,
               critique_model,
               research_model,
               stt_model,
               tts_model,
               tts_voice,
               tts_language,
               tts_speed,
               lite_model,
               updated_at
        FROM settings
        WHERE id = $1
        "#,
        Settings::SINGLETON_ID,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

async fn seed_singleton(pool: &PgPool) -> Result<(), AppError> {
    sqlx::query!(
        r#"
        INSERT INTO settings (
            id, critique_model, research_model, stt_model, tts_model,
            tts_voice, tts_language, tts_speed, lite_model
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, NULL, $8)
        ON CONFLICT (id) DO NOTHING
        "#,
        Settings::SINGLETON_ID,
        openrouter::DEFAULT_CRITIQUE_MODEL,
        openrouter::DEFAULT_RESEARCH_MODEL,
        openrouter::DEFAULT_STT_MODEL,
        openrouter::DEFAULT_TTS_MODEL,
        openrouter::DEFAULT_TTS_VOICE,
        "",
        openrouter::DEFAULT_LITE_MODEL,
    )
    .execute(pool)
    .await?;
    tracing::info!(event = "store.settings.seed", "seeded defaults singleton");
    Ok(())
}

pub async fn save(pool: &PgPool, s: &Settings) -> Result<(), AppError> {
    let now = chrono::Utc::now();
    sqlx::query!(
        r#"
        INSERT INTO settings (
            id, critique_model, research_model, stt_model, tts_model,
            tts_voice, tts_language, tts_speed, lite_model, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        ON CONFLICT (id) DO UPDATE
            SET critique_model = EXCLUDED.critique_model,
                research_model = EXCLUDED.research_model,
                stt_model      = EXCLUDED.stt_model,
                tts_model      = EXCLUDED.tts_model,
                tts_voice      = EXCLUDED.tts_voice,
                tts_language   = EXCLUDED.tts_language,
                tts_speed      = EXCLUDED.tts_speed,
                lite_model     = EXCLUDED.lite_model,
                updated_at     = EXCLUDED.updated_at
        "#,
        Settings::SINGLETON_ID,
        s.critique_model,
        s.research_model,
        s.stt_model,
        s.tts_model,
        s.tts_voice,
        s.tts_language,
        s.tts_speed,
        s.lite_model,
        now,
    )
    .execute(pool)
    .await?;
    tracing::info!(
        event = "store.settings.save",
        "settings singleton persisted"
    );
    Ok(())
}

/// Process-local cache for the Settings singleton.
///
/// Settings reads happen on every chat / lockin / curate / research call;
/// the underlying row mutates only when the user clicks Save on the
/// Settings page. We cache the whole row in an `Arc<RwLock<Option<_>>>`
/// (matching `BookCache` / `ModelsCache` style) and invalidate from the
/// save handler. Single-user, no TTL.
#[derive(Clone, Default)]
pub struct SettingsCache {
    inner: Arc<RwLock<Option<Settings>>>,
}

impl SettingsCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the cached Settings, loading on first read or after
    /// invalidation. Callers don't need to invalidate after [`save`] —
    /// the routes/settings handler does that.
    pub async fn get(&self, pool: &PgPool) -> Result<Settings, AppError> {
        {
            let guard = self.inner.read().await;
            if let Some(s) = guard.as_ref() {
                return Ok(s.clone());
            }
        }
        let fresh = load(pool).await?;
        let mut guard = self.inner.write().await;
        *guard = Some(fresh.clone());
        Ok(fresh)
    }

    /// Drop the cache. Called by `routes::settings::save` after writing.
    pub async fn invalidate(&self) {
        let mut guard = self.inner.write().await;
        *guard = None;
    }
}

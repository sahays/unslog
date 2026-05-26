//! Settings singleton: load-or-seed-defaults, save.

use std::sync::Arc;

use mongodb::Database;
use tokio::sync::RwLock;

use crate::error::AppError;
use crate::models::Settings;
use crate::services::openrouter;

/// Read the singleton, seeding compile-time defaults on first read.
pub async fn load(db: &Database) -> Result<Settings, AppError> {
    let coll = db.collection::<Settings>(Settings::COLLECTION);
    if let Some(s) = coll
        .find_one(bson::doc! { "_id": Settings::SINGLETON_ID })
        .await?
    {
        return Ok(s);
    }
    let seeded = Settings {
        id: Settings::SINGLETON_ID.to_string(),
        critique_model: openrouter::DEFAULT_CRITIQUE_MODEL.into(),
        research_model: openrouter::DEFAULT_RESEARCH_MODEL.into(),
        stt_model: openrouter::DEFAULT_STT_MODEL.into(),
        tts_model: openrouter::DEFAULT_TTS_MODEL.into(),
        tts_voice: openrouter::DEFAULT_TTS_VOICE.into(),
        tts_language: String::new(),
        tts_speed: None,
        lite_model: openrouter::DEFAULT_LITE_MODEL.into(),
        updated_at: chrono::Utc::now(),
    };
    coll.insert_one(&seeded).await?;
    Ok(seeded)
}

pub async fn save(db: &Database, s: &Settings) -> Result<(), AppError> {
    let coll = db.collection::<Settings>(Settings::COLLECTION);
    let mut next = s.clone();
    next.id = Settings::SINGLETON_ID.to_string();
    next.updated_at = chrono::Utc::now();
    coll.replace_one(bson::doc! { "_id": Settings::SINGLETON_ID }, &next)
        .upsert(true)
        .await?;
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
    pub async fn get(&self, db: &Database) -> Result<Settings, AppError> {
        {
            let guard = self.inner.read().await;
            if let Some(s) = guard.as_ref() {
                return Ok(s.clone());
            }
        }
        let fresh = load(db).await?;
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

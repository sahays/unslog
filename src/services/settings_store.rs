//! Settings singleton: load-or-seed-defaults, save.

use mongodb::Database;

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

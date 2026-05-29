//! Store for the `stories` collection — Story Builder shells.
//!
//! Read/write surface:
//! * `get` / `find_or_404` — lookups.
//! * `push_turn` — append chat turn, bump updated_at.
//! * `set_current_version` — repoint head + flip status to Complete.
//! * `set_status` / `set_mode` — small status / coach-mode flips.
//! * `delete_cascade` — wipe the story and all its versions (the only place
//!   that knows the parent–child shape).
//! * `list_for_landing` / `list_siblings` — projected reads for the index
//!   tile grid and the show-page sidebar.

use futures::TryStreamExt;
use mongodb::options::FindOptions;
use mongodb::Database;

use crate::error::AppError;
use crate::models::{ChatTurn, Difficulty, Story, StoryStatus};

/// Hard ceiling on the embedded chat array. Well above the ~50-turn
/// working set the UX is designed for, low enough that a runaway loop
/// can't grow a single Mongo doc past the 16 MB limit. Hitting the cap
/// surfaces a 400 telling the user to lock in and continue from there.
const MAX_CHAT_TURNS: usize = 100;

/// Pure capacity guard for [`push_turn`]. Returns `Err` when the chat
/// is already at `MAX_CHAT_TURNS`; otherwise `Ok(())`. Split out so we
/// can unit-test the cap without a live `&Database` handle.
fn check_chat_capacity(current_len: usize) -> Result<(), AppError> {
    if current_len >= MAX_CHAT_TURNS {
        return Err(AppError::BadRequest(format!(
            "chat is full ({MAX_CHAT_TURNS} turns) — lock this version in and refine from there"
        )));
    }
    Ok(())
}

/// Projected row used by the landing page tile grid + completed-card list.
/// We never need the embedded chat to render landing — keeping the
/// projection tight saves multi-KB per row.
#[derive(serde::Deserialize)]
pub struct StoryListRow {
    #[serde(rename = "_id")]
    pub id: String,
    pub competency_id: String,
    pub status: StoryStatus,
    #[serde(default)]
    pub current_version_id: Option<String>,
    #[serde(with = "crate::models::datetime_compat::required")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Sibling row used by the show-page side panel. Same projection idea as
/// `StoryListRow` but with story-id distinct fields and sibling-side
/// filtering (parent-scoped to one competency, excluding the current id).
pub struct SiblingRow {
    pub id: String,
    pub status: StoryStatus,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

pub async fn get(db: &Database, id: &str) -> Result<Option<Story>, AppError> {
    Ok(db
        .collection::<Story>(Story::COLLECTION)
        .find_one(bson::doc! { "_id": id })
        .await?)
}

pub async fn find_or_404(db: &Database, id: &str) -> Result<Story, AppError> {
    get(db, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("story {id}")))
}

pub async fn insert(db: &Database, story: &Story) -> Result<(), AppError> {
    db.collection::<Story>(Story::COLLECTION)
        .insert_one(story)
        .await?;
    Ok(())
}

/// Append a chat turn and bump `updated_at`. Mirrors
/// [`crate::services::pitch_store::push_turn`]. Refuses to grow the
/// embedded `chat[]` past [`MAX_CHAT_TURNS`] turns — the caller should
/// lock in a version and start a refine chat from a clean slate.
pub async fn push_turn(db: &Database, story: &mut Story, turn: ChatTurn) -> Result<(), AppError> {
    check_chat_capacity(story.chat.len())?;
    let now = chrono::Utc::now();
    let turn_doc = bson::to_bson(&turn)?;
    db.collection::<Story>(Story::COLLECTION)
        .update_one(
            bson::doc! { "_id": &story.id },
            bson::doc! {
                "$push": { "chat": turn_doc },
                "$set":  { "updated_at": now.to_rfc3339() },
            },
        )
        .await?;
    story.chat.push(turn);
    story.updated_at = now;
    Ok(())
}

/// Repoint `current_version_id` and flip status to `complete`. Called by
/// `story_lockin::generate_and_save` after inserting a fresh version.
pub async fn set_current_version(
    db: &Database,
    story_id: &str,
    version_id: &str,
) -> Result<(), AppError> {
    db.collection::<Story>(Story::COLLECTION)
        .update_one(
            bson::doc! { "_id": story_id },
            bson::doc! { "$set": {
                "current_version_id": version_id,
                "status": StoryStatus::Complete.as_str(),
                "updated_at": chrono::Utc::now().to_rfc3339(),
            } },
        )
        .await?;
    Ok(())
}

/// Flip status (without other side-effects). Used by `continue_chat` to
/// reopen a completed story for refinement, and could be used by other
/// status flips that don't touch the version pointer.
pub async fn set_status(
    db: &Database,
    story_id: &str,
    status: StoryStatus,
) -> Result<(), AppError> {
    db.collection::<Story>(Story::COLLECTION)
        .update_one(
            bson::doc! { "_id": story_id },
            bson::doc! { "$set": {
                "status": status.as_str(),
                "updated_at": chrono::Utc::now().to_rfc3339(),
            } },
        )
        .await?;
    Ok(())
}

/// Update coach mode for this story. New mode takes effect on the next
/// chat turn (which loads `story.mode` to pick the matching prompt).
pub async fn set_mode(db: &Database, story_id: &str, mode: Difficulty) -> Result<(), AppError> {
    db.collection::<Story>(Story::COLLECTION)
        .update_one(
            bson::doc! { "_id": story_id },
            bson::doc! { "$set": {
                "mode": mode.as_str(),
                "updated_at": chrono::Utc::now().to_rfc3339(),
            } },
        )
        .await?;
    Ok(())
}

/// Delete the story and every version it ever produced. The only place
/// that knows the parent–child shape, so callers don't need to.
pub async fn delete_cascade(db: &Database, story_id: &str) -> Result<(), AppError> {
    db.collection::<Story>(Story::COLLECTION)
        .delete_one(bson::doc! { "_id": story_id })
        .await?;
    crate::services::story_version_store::delete_for_story(db, story_id).await?;
    Ok(())
}

/// Projected list for the landing page — every story, newest-first.
pub async fn list_for_landing(db: &Database) -> Result<Vec<StoryListRow>, AppError> {
    let opts = FindOptions::builder()
        .sort(bson::doc! { "updated_at": -1 })
        .projection(bson::doc! {
            "_id": 1,
            "competency_id": 1,
            "status": 1,
            "current_version_id": 1,
            "updated_at": 1,
        })
        .build();
    let cursor = db
        .collection::<StoryListRow>(Story::COLLECTION)
        .find(bson::doc! {})
        .with_options(opts)
        .await?;
    Ok(cursor.try_collect().await?)
}

/// Other stories for the same competency, excluding `story_id`. Sorted by
/// most-recently-updated. Powers the side panel on the show page.
pub async fn list_siblings(
    db: &Database,
    story_id: &str,
    competency_id: &str,
) -> Result<Vec<SiblingRow>, AppError> {
    #[derive(serde::Deserialize)]
    struct Row {
        #[serde(rename = "_id")]
        id: String,
        status: StoryStatus,
        #[serde(with = "crate::models::datetime_compat::required")]
        updated_at: chrono::DateTime<chrono::Utc>,
    }
    let opts = FindOptions::builder()
        .sort(bson::doc! { "updated_at": -1 })
        .projection(bson::doc! { "_id": 1, "status": 1, "updated_at": 1 })
        .build();
    let cursor = db
        .collection::<Row>(Story::COLLECTION)
        .find(bson::doc! {
            "competency_id": competency_id,
            "_id": { "$ne": story_id },
        })
        .with_options(opts)
        .await?;
    let rows: Vec<Row> = cursor.try_collect().await?;
    Ok(rows
        .into_iter()
        .map(|r| SiblingRow {
            id: r.id,
            status: r.status,
            updated_at: r.updated_at,
        })
        .collect())
}

#[cfg(test)]
#[path = "story_store_tests.rs"]
mod tests;

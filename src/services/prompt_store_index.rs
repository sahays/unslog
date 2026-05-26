//! List-page aggregation for `prompt_store`.
//!
//! The list page renders an excerpt + a "vX of Y" label for every prompt
//! in `PROMPT_NAMES`. A naive per-name loop fans out to ~30 round-trips;
//! [`list_for_index`] collapses that to two `$in` queries (prompts +
//! prompt_versions) grouped in app code.

use mongodb::Database;

use crate::error::AppError;
use crate::models::{Prompt, PromptVersion};

/// Bulk-load prompt rows for the list page in one `$in` query, keyed by
/// `_id` (prompt name).
pub async fn get_all_by_names(
    db: &Database,
    names: &[&str],
) -> Result<std::collections::HashMap<String, Prompt>, AppError> {
    use futures::TryStreamExt;
    if names.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let cursor = db
        .collection::<Prompt>(Prompt::COLLECTION)
        .find(bson::doc! { "_id": { "$in": names } })
        .await?;
    let rows: Vec<Prompt> = cursor.try_collect().await?;
    Ok(rows.into_iter().map(|p| (p.name.clone(), p)).collect())
}

/// One projected row used by the list page — populated by
/// [`list_for_index`].
pub struct ListRow {
    pub total_n: u32,
    pub active_n: u32,
    pub active_body: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// `name → ListRow` for the list page. One round-trip per collection;
/// missing prompts are silently skipped (the caller emits a "never-
/// seeded" placeholder card).
pub async fn list_for_index(
    db: &Database,
    names: &[&str],
) -> Result<std::collections::HashMap<String, ListRow>, AppError> {
    use futures::TryStreamExt;
    use mongodb::options::FindOptions;
    let prompts = get_all_by_names(db, names).await?;
    if prompts.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    #[derive(serde::Deserialize)]
    struct VRow {
        #[serde(rename = "_id")]
        id: String,
        prompt_name: String,
        body: String,
    }
    let opts = FindOptions::builder()
        .sort(bson::doc! { "created_at": -1 })
        .projection(bson::doc! { "_id": 1, "prompt_name": 1, "body": 1 })
        .build();
    let rows: Vec<VRow> = db
        .collection::<VRow>(PromptVersion::COLLECTION)
        .find(bson::doc! { "prompt_name": { "$in": names } })
        .with_options(opts)
        .await?
        .try_collect()
        .await?;
    let mut grouped: std::collections::HashMap<String, Vec<VRow>> =
        std::collections::HashMap::new();
    for r in rows {
        grouped.entry(r.prompt_name.clone()).or_default().push(r);
    }
    let mut out = std::collections::HashMap::with_capacity(prompts.len());
    for (name, prompt) in prompts {
        let versions = grouped.remove(&name).unwrap_or_default();
        let total_n = versions.len() as u32;
        // active_n = chronological number of the row whose id matches
        // current_version_id, where vector is newest-first.
        let active_n = versions
            .iter()
            .position(|v| v.id == prompt.current_version_id)
            .map(|idx| (total_n as usize - idx) as u32)
            .unwrap_or(0);
        let active_body = versions
            .into_iter()
            .find(|v| v.id == prompt.current_version_id)
            .map(|v| v.body)
            .unwrap_or_default();
        out.insert(
            name,
            ListRow {
                total_n,
                active_n,
                active_body,
                updated_at: prompt.updated_at,
            },
        );
    }
    Ok(out)
}

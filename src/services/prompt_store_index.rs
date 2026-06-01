//! List-page aggregation for `prompt_store`.
//!
//! The list page renders an excerpt + a "vX of Y" label for every prompt
//! in `PROMPT_NAMES`. A naive per-name loop fans out to N round-trips;
//! [`list_for_index`] collapses that to two queries (prompts + versions)
//! grouped in app code.

use std::collections::HashMap;

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::Prompt;

/// Bulk-load prompt rows for the list page in one `WHERE name = ANY($1)`
/// query, keyed by `name`.
pub async fn get_all_by_names(
    pool: &PgPool,
    names: &[&str],
) -> Result<HashMap<String, Prompt>, AppError> {
    if names.is_empty() {
        return Ok(HashMap::new());
    }
    let owned: Vec<String> = names.iter().map(|s| (*s).to_string()).collect();
    let rows = sqlx::query_as!(
        Prompt,
        r#"
        SELECT name, current_version_id, updated_at
        FROM prompts
        WHERE name = ANY($1)
        "#,
        &owned,
    )
    .fetch_all(pool)
    .await?;
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

/// Compact `(id, body)` projection of `prompt_versions` — kept narrow so
/// the index page never pulls full bodies it doesn't need. `prompt_name`
/// is used only to bucket rows during the SQL → app pivot and is dropped
/// before the row hits the struct.
struct VRow {
    id: String,
    body: String,
}

/// `name → ListRow` for the list page. Two round-trips total; missing
/// prompts are silently skipped (the caller emits a "never-seeded"
/// placeholder card).
pub async fn list_for_index(
    pool: &PgPool,
    names: &[&str],
) -> Result<HashMap<String, ListRow>, AppError> {
    let prompts = get_all_by_names(pool, names).await?;
    if prompts.is_empty() {
        return Ok(HashMap::new());
    }
    let grouped = load_versions_grouped(pool, names).await?;
    Ok(assemble_index_rows(prompts, grouped))
}

async fn load_versions_grouped(
    pool: &PgPool,
    names: &[&str],
) -> Result<HashMap<String, Vec<VRow>>, AppError> {
    let owned: Vec<String> = names.iter().map(|s| (*s).to_string()).collect();
    let rows = sqlx::query!(
        r#"
        SELECT id, prompt_name, body
        FROM prompt_versions
        WHERE prompt_name = ANY($1)
        ORDER BY created_at DESC
        "#,
        &owned,
    )
    .fetch_all(pool)
    .await?;
    let mut grouped: HashMap<String, Vec<VRow>> = HashMap::new();
    for r in rows {
        grouped.entry(r.prompt_name).or_default().push(VRow {
            id: r.id,
            body: r.body,
        });
    }
    Ok(grouped)
}

fn assemble_index_rows(
    prompts: HashMap<String, Prompt>,
    mut grouped: HashMap<String, Vec<VRow>>,
) -> HashMap<String, ListRow> {
    let mut out = HashMap::with_capacity(prompts.len());
    for (name, prompt) in prompts {
        let versions = grouped.remove(&name).unwrap_or_default();
        out.insert(name, build_list_row(&prompt, versions));
    }
    out
}

fn build_list_row(prompt: &Prompt, versions: Vec<VRow>) -> ListRow {
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
    ListRow {
        total_n,
        active_n,
        active_body,
        updated_at: prompt.updated_at,
    }
}

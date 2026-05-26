//! `GET /agents` — the index card grid showing every known prompt with a
//! body excerpt and "vN of M" label. Uses the bulk `list_for_index` store
//! call so we hit Mongo with two queries (prompts + prompt_versions) not
//! one per prompt.

use askama::Template;
use axum::extract::State;
use axum::response::Html;

use super::{describe, excerpt};
use crate::error::AppError;
use crate::filters;
use crate::models::PROMPT_NAMES;
use crate::services::prompt_store as store;
use crate::startup::AppState;

#[derive(Template)]
#[template(path = "prompts/list.html")]
struct ListTemplate {
    items: Vec<PromptCard>,
}

struct PromptCard {
    name: &'static str,
    description: &'static str,
    body_excerpt: String,
    /// Display number of the active version (1-based, in chronological order).
    /// Zero only on never-seeded prompts that shouldn't appear in PROMPT_NAMES.
    active_n: u32,
    /// Total number of versions ever saved for this prompt.
    total_n: u32,
    /// When the prompt was last updated (a new version was activated).
    /// `None` for never-seeded prompts.
    updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub(super) async fn list(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let names = PROMPT_NAMES.to_vec();
    let rows = store::list_for_index(&state.db, &names).await?;
    let items: Vec<PromptCard> = PROMPT_NAMES
        .iter()
        .map(|name| card_for(name, &rows))
        .collect();
    crate::error::render_html(ListTemplate { items })
}

fn card_for(
    name: &&'static str,
    rows: &std::collections::HashMap<String, store::ListRow>,
) -> PromptCard {
    match rows.get(*name) {
        Some(r) => PromptCard {
            name,
            description: describe(name),
            body_excerpt: excerpt(&r.active_body, 280),
            active_n: r.active_n,
            total_n: r.total_n,
            updated_at: Some(r.updated_at),
        },
        None => PromptCard {
            name,
            description: describe(name),
            body_excerpt: String::new(),
            active_n: 0,
            total_n: 0,
            updated_at: None,
        },
    }
}

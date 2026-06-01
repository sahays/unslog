//! Canonical category list — seeded on first run, editable later.
//!
//! Backed by Postgres `categories` table after Phase A. The seed list also
//! drives the display sort_order so the candidate sees the same canonical
//! ordering every launch.

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::Category;

/// Twelve canonical behavioral-interview competencies.
/// Index 0–11 maps directly to sort_order so the seed list also drives display order.
const SEED: &[(&str, &str, &str)] = &[
    (
        "ownership",
        "Ownership",
        "Drove an outcome end-to-end; took accountability when things went wrong.",
    ),
    (
        "prioritization",
        "Prioritization",
        "Made hard tradeoffs under constraint; chose what to drop or postpone.",
    ),
    (
        "bias_for_action",
        "Bias for Action",
        "Moved decisively with incomplete information; reversed when wrong.",
    ),
    (
        "dealing_with_ambiguity",
        "Dealing with Ambiguity",
        "Operated without clear direction; defined the problem and a path forward.",
    ),
    (
        "customer_obsession",
        "Customer / User Obsession",
        "Worked backwards from real user needs; pushed back on internal preferences.",
    ),
    (
        "collaboration_influence",
        "Collaboration & Influence",
        "Got cross-team or cross-org work done without formal authority.",
    ),
    (
        "conflict_disagreement",
        "Conflict & Disagreement",
        "Disagreed productively; held a position under pressure or yielded for the right reason.",
    ),
    (
        "communication",
        "Communication",
        "Explained complex ideas clearly to mixed audiences; wrote or spoke to drive a decision.",
    ),
    (
        "highest_standards",
        "Highest Standards",
        "Refused sloppy work; raised the bar on quality, craft, or rigour.",
    ),
    (
        "diving_deep",
        "Diving Deep",
        "Went past the surface; root-caused; got into data or technical details.",
    ),
    (
        "strategic_thinking",
        "Strategic Thinking",
        "Long-term vision, frameworks, second-order consequences.",
    ),
    (
        "failure_learning",
        "Failure & Learning",
        "Recovered from a failure; extracted lessons; applied them later.",
    ),
];

/// On startup, ensure the canonical category list is in place. Insert all 12
/// seed rows only on first boot — `ON CONFLICT (id) DO NOTHING` makes the
/// seed idempotent and preserves any user edits to existing rows.
pub async fn seed_defaults(pool: &PgPool) -> Result<(), AppError> {
    let mut inserted = 0_usize;
    for (i, (id, name, description)) in SEED.iter().enumerate() {
        let sort_order = i as i32;
        let rows = sqlx::query!(
            r#"
            INSERT INTO categories (id, name, description, sort_order)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (id) DO NOTHING
            "#,
            id,
            name,
            description,
            sort_order,
        )
        .execute(pool)
        .await?
        .rows_affected();
        inserted += rows as usize;
    }
    tracing::info!(
        event = "store.categories.seed",
        inserted,
        total = SEED.len(),
        "seeded canonical categories"
    );
    Ok(())
}

pub async fn list_all(pool: &PgPool) -> Result<Vec<Category>, AppError> {
    let rows = sqlx::query_as!(
        Category,
        r#"
        SELECT id, name, description, sort_order, created_at
        FROM categories
        ORDER BY sort_order, name
        "#
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get(pool: &PgPool, id: &str) -> Result<Option<Category>, AppError> {
    let row = sqlx::query_as!(
        Category,
        r#"
        SELECT id, name, description, sort_order, created_at
        FROM categories
        WHERE id = $1
        "#,
        id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Upsert by primary key. Updates name/description/sort_order on existing
/// rows; created_at stays at the original value.
pub async fn save(pool: &PgPool, cat: &Category) -> Result<(), AppError> {
    sqlx::query!(
        r#"
        INSERT INTO categories (id, name, description, sort_order, created_at)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (id) DO UPDATE
            SET name = EXCLUDED.name,
                description = EXCLUDED.description,
                sort_order = EXCLUDED.sort_order
        "#,
        cat.id,
        cat.name,
        cat.description,
        cat.sort_order,
        cat.created_at,
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete(pool: &PgPool, id: &str) -> Result<(), AppError> {
    sqlx::query!("DELETE FROM categories WHERE id = $1", id)
        .execute(pool)
        .await?;
    Ok(())
}

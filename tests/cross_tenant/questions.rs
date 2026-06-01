//! Cross-tenant isolation for `services::questions`.
//!
//! Owner-only questions are isolated. Role-only questions
//! (`company_id IS NULL`) are owner-scoped too — the curator pools
//! across the *caller's* row set, never across users' rows.

use sqlx::PgPool;

use unslog::models::Role;
use unslog::services::questions;

use crate::cross_tenant::common::{insert_company, insert_question, setup_two_users};

#[sqlx::test(migrations = "./migrations")]
async fn case_get_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let co = insert_company(&pool, &users.alice, "Acme").await;
    let q = insert_question(&pool, &users.alice, Some(co.id.clone()), "alice-q?").await;

    let bobs = questions::get(&pool, &users.bob.id, &q.id)
        .await
        .expect("get");
    assert!(bobs.is_none(), "Bob must not see Alice's question");
    let alices = questions::get(&pool, &users.alice.id, &q.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(alices.id, q.id);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_list_for_company_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let co = insert_company(&pool, &users.alice, "Acme").await;
    let _ = insert_question(&pool, &users.alice, Some(co.id.clone()), "alice-q?").await;

    let bobs = questions::list_for_company(&pool, &users.bob.id, &co.id)
        .await
        .expect("list");
    assert!(
        bobs.is_empty(),
        "Bob's list for Alice's company id is empty"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn case_list_for_pool_isolated_for_company_questions(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let co = insert_company(&pool, &users.alice, "Acme").await;
    let _ = insert_question(&pool, &users.alice, Some(co.id.clone()), "alice-co-q?").await;

    // Bob's pool with Alice's company id: still empty — questions are
    // owner-scoped.
    let bobs = questions::list_for_pool(
        &pool,
        &users.bob.id,
        Role::SoftwareEngineer,
        std::slice::from_ref(&co.id),
    )
    .await
    .expect("pool");
    assert!(
        bobs.is_empty(),
        "Bob's pool must not include Alice's tagged questions"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn case_list_for_pool_role_only_question_is_per_owner(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    // Role-only question (no company_id), owned by Alice.
    let _ = insert_question(&pool, &users.alice, None, "role-only-q?").await;

    let bobs = questions::list_for_pool(&pool, &users.bob.id, Role::SoftwareEngineer, &[])
        .await
        .expect("pool");
    assert!(
        bobs.is_empty(),
        "role-only questions are still owner-scoped; Bob sees none of Alice's"
    );

    let alices = questions::list_for_pool(&pool, &users.alice.id, Role::SoftwareEngineer, &[])
        .await
        .expect("pool");
    assert_eq!(alices.len(), 1, "Alice still sees her role-only question");
}

#[sqlx::test(migrations = "./migrations")]
async fn case_update_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let co = insert_company(&pool, &users.alice, "Acme").await;
    let q = insert_question(&pool, &users.alice, Some(co.id.clone()), "alice-orig?").await;

    questions::update(
        &pool,
        &users.bob.id,
        &q.id,
        "bob-stole?",
        Role::EngineeringManager,
        &[],
    )
    .await
    .expect("ok with 0 rows affected");

    let still = questions::get(&pool, &users.alice.id, &q.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(still.text, "alice-orig?");
}

#[sqlx::test(migrations = "./migrations")]
async fn case_delete_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let co = insert_company(&pool, &users.alice, "Acme").await;
    let q = insert_question(&pool, &users.alice, Some(co.id.clone()), "alice-q?").await;

    questions::delete(&pool, &users.bob.id, &q.id)
        .await
        .expect("ok");

    let still = questions::get(&pool, &users.alice.id, &q.id)
        .await
        .expect("get");
    assert!(
        still.is_some(),
        "Bob's delete must not remove Alice's question"
    );
}

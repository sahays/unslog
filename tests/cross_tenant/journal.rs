//! Cross-tenant isolation for `services::journal_store`.

use sqlx::PgPool;

use unslog::services::journal_store;

use crate::cross_tenant::common::{insert_journal, setup_two_users};

#[sqlx::test(migrations = "./migrations")]
async fn case_get_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_je = insert_journal(&pool, &users.alice, "memo").await;

    let bobs = journal_store::get(&pool, &users.bob.id, &alice_je.id)
        .await
        .expect("get");
    assert!(bobs.is_none());
    let alices = journal_store::get(&pool, &users.alice.id, &alice_je.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(alices.id, alice_je.id);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_list_active_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let _alice_je = insert_journal(&pool, &users.alice, "alice-only").await;
    let _bob_je = insert_journal(&pool, &users.bob, "bob-only").await;

    let bob_list = journal_store::list_active(&pool, &users.bob.id)
        .await
        .expect("list");
    let titles: Vec<String> = bob_list.iter().map(|j| j.title.clone()).collect();
    assert_eq!(titles, vec!["bob-only".to_string()]);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_update_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_je = insert_journal(&pool, &users.alice, "alice-orig").await;

    let err = journal_store::update(
        &pool,
        &users.bob.id,
        &alice_je.id,
        "bob-stole".into(),
        "x".into(),
    )
    .await;
    assert!(
        matches!(err, Err(unslog::error::AppError::NotFound(_))),
        "Bob's update of Alice's row must surface as NotFound, got {err:?}"
    );

    let still = journal_store::get(&pool, &users.alice.id, &alice_je.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(still.title, "alice-orig");
}

#[sqlx::test(migrations = "./migrations")]
async fn case_archive_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_je = insert_journal(&pool, &users.alice, "alice-active").await;

    let err = journal_store::archive(&pool, &users.bob.id, &alice_je.id).await;
    assert!(
        matches!(err, Err(unslog::error::AppError::NotFound(_))),
        "Bob's archive of Alice's row must surface as NotFound, got {err:?}"
    );

    // Alice's row is still active.
    let still = journal_store::get(&pool, &users.alice.id, &alice_je.id)
        .await
        .expect("get")
        .expect("present");
    assert!(still.archived_at.is_none());
}

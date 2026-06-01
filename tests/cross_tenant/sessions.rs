//! Cross-tenant isolation for `services::sessions`.

use sqlx::PgPool;

use unslog::services::sessions;

use crate::cross_tenant::common::{insert_company, insert_session, setup_two_users};

#[sqlx::test(migrations = "./migrations")]
async fn case_get_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_co = insert_company(&pool, &users.alice, "Acme").await;
    let alice_sess = insert_session(&pool, &users.alice, &alice_co).await;

    let bobs = sessions::get(&pool, &users.bob.id, &alice_sess.id)
        .await
        .expect("get");
    assert!(bobs.is_none());
    let alices = sessions::get(&pool, &users.alice.id, &alice_sess.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(alices.id, alice_sess.id);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_list_active_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_co = insert_company(&pool, &users.alice, "Acme").await;
    let bob_co = insert_company(&pool, &users.bob, "Beta").await;
    let _alice_sess = insert_session(&pool, &users.alice, &alice_co).await;
    let bob_sess = insert_session(&pool, &users.bob, &bob_co).await;

    let bobs = sessions::list_active(&pool, &users.bob.id)
        .await
        .expect("list");
    let ids: Vec<String> = bobs.iter().map(|s| s.id.clone()).collect();
    assert_eq!(ids, vec![bob_sess.id]);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_list_for_company_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_co = insert_company(&pool, &users.alice, "Acme").await;
    let _ = insert_session(&pool, &users.alice, &alice_co).await;

    // Bob lists sessions for Alice's company id under his owner. Empty.
    let bob_view = sessions::list_for_company(&pool, &users.bob.id, &alice_co.id)
        .await
        .expect("list");
    assert!(
        bob_view.is_empty(),
        "Bob must see no sessions in Alice's company"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn case_count_active_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_co = insert_company(&pool, &users.alice, "Acme").await;
    let _ = insert_session(&pool, &users.alice, &alice_co).await;

    let bobn = sessions::count_active(&pool, &users.bob.id)
        .await
        .expect("count");
    assert_eq!(bobn, 0);
    let alicen = sessions::count_active(&pool, &users.alice.id)
        .await
        .expect("count");
    assert_eq!(alicen, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_set_current_question_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_co = insert_company(&pool, &users.alice, "Acme").await;
    let alice_sess = insert_session(&pool, &users.alice, &alice_co).await;

    sessions::set_current_question(
        &pool,
        &users.bob.id,
        &alice_sess.id,
        "qst000001",
        "what?",
        None,
    )
    .await
    .expect("ok with 0 rows affected");

    let still = sessions::get(&pool, &users.alice.id, &alice_sess.id)
        .await
        .expect("get")
        .expect("present");
    assert!(still.current_question_id.is_none());
    assert!(still.current_question_text.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn case_end_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_co = insert_company(&pool, &users.alice, "Acme").await;
    let alice_sess = insert_session(&pool, &users.alice, &alice_co).await;

    sessions::end(&pool, &users.bob.id, &alice_sess.id)
        .await
        .expect("ok");

    let still = sessions::get(&pool, &users.alice.id, &alice_sess.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(still.status, unslog::models::SessionStatus::Active);
    assert!(still.ended_at.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn case_delete_cascade_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_co = insert_company(&pool, &users.alice, "Acme").await;
    let alice_sess = insert_session(&pool, &users.alice, &alice_co).await;

    let n = sessions::delete_cascade(&pool, &users.bob.id, &alice_sess.id)
        .await
        .expect("delete");
    assert_eq!(n, 0, "Bob's delete must affect 0 rows");

    let still = sessions::get(&pool, &users.alice.id, &alice_sess.id)
        .await
        .expect("get");
    assert!(still.is_some(), "session must survive Bob's attempt");
}

#[sqlx::test(migrations = "./migrations")]
async fn case_toggle_voice_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_co = insert_company(&pool, &users.alice, "Acme").await;
    let alice_sess = insert_session(&pool, &users.alice, &alice_co).await;

    // Bob clones Alice's session and tries to toggle voice — the WHERE
    // owner_id = bob filter matches 0 rows; flag stays as it was.
    let spoofed = unslog::models::Session {
        owner_id: users.bob.id.clone(),
        ..alice_sess.clone()
    };
    let _ = sessions::toggle_voice(&pool, &spoofed).await.expect("ok");

    let still = sessions::get(&pool, &users.alice.id, &alice_sess.id)
        .await
        .expect("get")
        .expect("present");
    assert!(!still.voice_critique_enabled, "flag stayed at false");
}

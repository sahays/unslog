//! Cross-tenant isolation for `services::summary` (store layer).

use sqlx::PgPool;

use unslog::services::summary;

use crate::cross_tenant::common::{
    insert_company, insert_session, insert_summary, setup_two_users,
};

#[sqlx::test(migrations = "./migrations")]
async fn case_for_session_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_co = insert_company(&pool, &users.alice, "Acme").await;
    let alice_sess = insert_session(&pool, &users.alice, &alice_co).await;
    let _ = insert_summary(&pool, &users.alice, &alice_sess, &alice_co).await;

    let bobs = summary::for_session(&pool, &users.bob.id, &alice_sess.id)
        .await
        .expect("for_session");
    assert!(bobs.is_none(), "Bob must not see Alice's summary");

    let alices = summary::for_session(&pool, &users.alice.id, &alice_sess.id)
        .await
        .expect("for_session")
        .expect("present");
    assert_eq!(alices.session_id, alice_sess.id);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_by_session_ids_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_co = insert_company(&pool, &users.alice, "Acme").await;
    let alice_sess = insert_session(&pool, &users.alice, &alice_co).await;
    let _ = insert_summary(&pool, &users.alice, &alice_sess, &alice_co).await;

    let ids: Vec<&str> = vec![alice_sess.id.as_str()];
    let map = summary::by_session_ids(&pool, &users.bob.id, &ids)
        .await
        .expect("by_session_ids");
    assert!(map.is_empty(), "Bob's bulk-load must drop Alice's session");
}

#[sqlx::test(migrations = "./migrations")]
async fn case_recent_company_summaries_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_co = insert_company(&pool, &users.alice, "Acme").await;
    let alice_sess = insert_session(&pool, &users.alice, &alice_co).await;
    let _ = insert_summary(&pool, &users.alice, &alice_sess, &alice_co).await;

    let bobs = summary::recent_company_summaries(&pool, &users.bob.id, &alice_co.id, None, 10)
        .await
        .expect("recent");
    assert!(
        bobs.is_empty(),
        "Bob's recent-company read must drop Alice's summaries"
    );
}

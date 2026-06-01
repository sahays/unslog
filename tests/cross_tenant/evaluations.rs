//! Cross-tenant isolation for `services::evaluations`.

use sqlx::PgPool;

use unslog::services::evaluations::{self, EvalSource, PgEvalSource};

use crate::cross_tenant::common::{
    insert_company, insert_evaluation, insert_question, insert_session, setup_two_users,
};

#[sqlx::test(migrations = "./migrations")]
async fn case_find_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_co = insert_company(&pool, &users.alice, "Acme").await;
    let alice_sess = insert_session(&pool, &users.alice, &alice_co).await;
    let q = insert_question(&pool, &users.alice, Some(alice_co.id.clone()), "q?").await;
    let _eval = insert_evaluation(&pool, &users.alice, &alice_sess, &q).await;

    let src = PgEvalSource { pool: &pool };
    let bobs = src
        .find(&users.bob.id, &alice_sess.id, &q.id)
        .await
        .expect("find");
    assert!(bobs.is_none(), "Bob must not see Alice's eval");

    let alices = src
        .find(&users.alice.id, &alice_sess.id, &q.id)
        .await
        .expect("find")
        .expect("present");
    assert_eq!(alices.session_id, alice_sess.id);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_list_for_session_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_co = insert_company(&pool, &users.alice, "Acme").await;
    let alice_sess = insert_session(&pool, &users.alice, &alice_co).await;
    let q = insert_question(&pool, &users.alice, Some(alice_co.id.clone()), "q?").await;
    let _eval = insert_evaluation(&pool, &users.alice, &alice_sess, &q).await;

    let bobs = evaluations::list_for_session(&pool, &users.bob.id, &alice_sess.id)
        .await
        .expect("list");
    assert!(bobs.is_empty(), "Bob sees no evals in Alice's session");

    let alices = evaluations::list_for_session(&pool, &users.alice.id, &alice_sess.id)
        .await
        .expect("list");
    assert_eq!(alices.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_counts_by_session_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_co = insert_company(&pool, &users.alice, "Acme").await;
    let alice_sess = insert_session(&pool, &users.alice, &alice_co).await;
    let q = insert_question(&pool, &users.alice, Some(alice_co.id.clone()), "q?").await;
    let _eval = insert_evaluation(&pool, &users.alice, &alice_sess, &q).await;

    let ids: Vec<&str> = vec![alice_sess.id.as_str()];
    let counts = evaluations::counts_by_session(&pool, &users.bob.id, &ids)
        .await
        .expect("counts");
    assert!(counts.is_empty(), "Bob's counts must drop Alice's session");
}

#[sqlx::test(migrations = "./migrations")]
async fn case_answered_question_ids_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_co = insert_company(&pool, &users.alice, "Acme").await;
    let alice_sess = insert_session(&pool, &users.alice, &alice_co).await;
    let q = insert_question(&pool, &users.alice, Some(alice_co.id.clone()), "q?").await;
    let _eval = insert_evaluation(&pool, &users.alice, &alice_sess, &q).await;

    let bobs = evaluations::answered_question_ids(&pool, &users.bob.id, &alice_sess.id)
        .await
        .expect("ids");
    assert!(bobs.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn case_load_for_attempt_edit_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_co = insert_company(&pool, &users.alice, "Acme").await;
    let alice_sess = insert_session(&pool, &users.alice, &alice_co).await;
    let q = insert_question(&pool, &users.alice, Some(alice_co.id.clone()), "q?").await;
    let eval = insert_evaluation(&pool, &users.alice, &alice_sess, &q).await;

    let bobs =
        evaluations::load_for_attempt_edit(&pool, &users.bob.id, &alice_sess.id, &eval.id).await;
    assert!(
        matches!(bobs, Err(unslog::error::AppError::NotFound(_))),
        "Bob's attempt-edit load must 404, got {bobs:?}"
    );
}

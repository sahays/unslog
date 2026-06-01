//! Cross-tenant isolation for `services::company_store`.
//!
//! Asserts the SQL filter on `owner_id` actually scopes results — Bob
//! cannot read, update, or delete Alice's company rows even with the id.

use sqlx::PgPool;

use unslog::models::ResearchPacket;
use unslog::services::company_store;

use crate::cross_tenant::common::{insert_company, setup_two_users};

#[sqlx::test(migrations = "./migrations")]
async fn case_get_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_co = insert_company(&pool, &users.alice, "Acme").await;

    let bob_view = company_store::get(&pool, &users.bob.id, &alice_co.id)
        .await
        .expect("get");
    assert!(bob_view.is_none(), "Bob must not see Alice's company by id");

    let alice_view = company_store::get(&pool, &users.alice.id, &alice_co.id)
        .await
        .expect("get");
    assert!(alice_view.is_some(), "Alice still sees her own row");
}

#[sqlx::test(migrations = "./migrations")]
async fn case_list_sorted_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let _ = insert_company(&pool, &users.alice, "AlphaCo").await;
    let _ = insert_company(&pool, &users.bob, "BetaCo").await;

    let bobs = company_store::list_sorted(&pool, &users.bob.id)
        .await
        .expect("list");
    let names: Vec<String> = bobs.iter().map(|c| c.name.clone()).collect();
    assert_eq!(names, vec!["BetaCo".to_string()]);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_find_by_ids_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_co = insert_company(&pool, &users.alice, "AlphaCo").await;

    let found =
        company_store::find_by_ids(&pool, &users.bob.id, std::slice::from_ref(&alice_co.id))
            .await
            .expect("find_by_ids");
    assert!(found.is_empty(), "Bob's find_by_ids must drop Alice's id");
}

#[sqlx::test(migrations = "./migrations")]
async fn case_update_packet_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_co = insert_company(&pool, &users.alice, "AlphaCo").await;

    let packet = ResearchPacket {
        summary: "BOB-was-here".into(),
        role_jd: String::new(),
        values_signal: String::new(),
        interview_process: String::new(),
        sample_questions: vec![],
        sources: vec![],
        fetched_urls: vec![],
        research_prompt_version_id: "prv000001".into(),
        last_refreshed_at: chrono::Utc::now(),
    };
    company_store::update_packet(&pool, &users.bob.id, &alice_co.id, &packet)
        .await
        .expect("update returns ok even when 0 rows match");

    let still = company_store::get(&pool, &users.alice.id, &alice_co.id)
        .await
        .expect("get")
        .expect("present");
    assert!(
        still.research_packet.is_none(),
        "Bob's update must not have leaked into Alice's row"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn case_delete_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_co = insert_company(&pool, &users.alice, "AlphaCo").await;

    company_store::delete(&pool, &users.bob.id, &alice_co.id)
        .await
        .expect("ok even with 0 rows");

    let still = company_store::get(&pool, &users.alice.id, &alice_co.id)
        .await
        .expect("get")
        .expect("still present");
    assert_eq!(still.id, alice_co.id);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_names_by_ids_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_co = insert_company(&pool, &users.alice, "AlphaCo").await;

    let ids: Vec<&str> = vec![alice_co.id.as_str()];
    let map = company_store::names_by_ids(&pool, &users.bob.id, &ids)
        .await
        .expect("names");
    assert!(map.is_empty(), "Bob sees no names for Alice's ids");
}

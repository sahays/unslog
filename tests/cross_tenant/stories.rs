//! Cross-tenant isolation for `services::story_store` +
//! `services::story_version_store`.

use sqlx::PgPool;

use unslog::models::{ChatRole, ChatTurn, Difficulty, StoryBody, StoryStatus};
use unslog::services::{story_store, story_version_store};

use crate::cross_tenant::common::{insert_story, setup_two_users};

const COMPETENCY: &str = "ownership";

fn turn(role: ChatRole, content: &str) -> ChatTurn {
    ChatTurn {
        role,
        content: content.into(),
        ts: chrono::Utc::now(),
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn case_get_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_story = insert_story(&pool, &users.alice, COMPETENCY).await;

    let bobs = story_store::get(&pool, &users.bob.id, &alice_story.id)
        .await
        .expect("get");
    assert!(bobs.is_none());
    let alices = story_store::get(&pool, &users.alice.id, &alice_story.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(alices.id, alice_story.id);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_list_for_landing_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let _alice = insert_story(&pool, &users.alice, COMPETENCY).await;
    let bob_story = insert_story(&pool, &users.bob, COMPETENCY).await;

    let bobs = story_store::list_for_landing(&pool, &users.bob.id)
        .await
        .expect("list");
    let ids: Vec<String> = bobs.iter().map(|r| r.id.clone()).collect();
    assert_eq!(ids, vec![bob_story.id]);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_list_in_progress_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let _alice = insert_story(&pool, &users.alice, COMPETENCY).await;

    let bobs = story_store::list_in_progress(&pool, &users.bob.id)
        .await
        .expect("list");
    assert!(bobs.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn case_list_siblings_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_a = insert_story(&pool, &users.alice, COMPETENCY).await;
    let _alice_b = insert_story(&pool, &users.alice, COMPETENCY).await;

    let bob_siblings = story_store::list_siblings(&pool, &users.bob.id, &alice_a.id, COMPETENCY)
        .await
        .expect("siblings");
    assert!(
        bob_siblings.is_empty(),
        "Bob's sibling query must not surface Alice's stories"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn case_push_turn_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_story = insert_story(&pool, &users.alice, COMPETENCY).await;

    // Bob spoofs Alice's id on the in-memory struct; SQL filter on
    // owner_id matches 0 rows.
    let mut spoofed = alice_story.clone();
    spoofed.owner_id = users.bob.id.clone();
    story_store::push_turn(&pool, &mut spoofed, turn(ChatRole::User, "boom"))
        .await
        .expect("ok with 0 rows affected");

    let still = story_store::get(&pool, &users.alice.id, &alice_story.id)
        .await
        .expect("get")
        .expect("present");
    assert!(still.chat.is_empty(), "Bob's push_turn must not write");
}

#[sqlx::test(migrations = "./migrations")]
async fn case_set_current_version_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_story = insert_story(&pool, &users.alice, COMPETENCY).await;
    // Alice inserts a version on her story; this is the legit precondition.
    let v = story_version_store::insert_with_next_n(
        &pool,
        &users.alice.id,
        &alice_story.id,
        &StoryBody::default(),
        None,
    )
    .await
    .expect("insert version");

    // Bob tries to repoint Alice's story to his "version". 0 rows match.
    story_store::set_current_version(&pool, &users.bob.id, &alice_story.id, &v.id)
        .await
        .expect("ok with 0 rows");

    let still = story_store::get(&pool, &users.alice.id, &alice_story.id)
        .await
        .expect("get")
        .expect("present");
    assert!(
        still.current_version_id.is_none(),
        "Bob's set_current_version must not have touched Alice's story"
    );
    assert_eq!(still.status, StoryStatus::InProgress);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_set_status_and_mode_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_story = insert_story(&pool, &users.alice, COMPETENCY).await;

    story_store::set_status(&pool, &users.bob.id, &alice_story.id, StoryStatus::Complete)
        .await
        .expect("ok");
    story_store::set_mode(
        &pool,
        &users.bob.id,
        &alice_story.id,
        Difficulty::Collaborative,
    )
    .await
    .expect("ok");

    let still = story_store::get(&pool, &users.alice.id, &alice_story.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(still.status, StoryStatus::InProgress);
    assert_eq!(still.mode, Difficulty::Strict);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_delete_cascade_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_story = insert_story(&pool, &users.alice, COMPETENCY).await;

    story_store::delete_cascade(&pool, &users.bob.id, &alice_story.id)
        .await
        .expect("ok");

    let still = story_store::get(&pool, &users.alice.id, &alice_story.id)
        .await
        .expect("get");
    assert!(
        still.is_some(),
        "Bob's delete must not remove Alice's story"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn case_version_get_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_story = insert_story(&pool, &users.alice, COMPETENCY).await;
    let v = story_version_store::insert_with_next_n(
        &pool,
        &users.alice.id,
        &alice_story.id,
        &StoryBody::default(),
        None,
    )
    .await
    .expect("insert version");

    let bobs = story_version_store::get(&pool, &users.bob.id, &v.id)
        .await
        .expect("get");
    assert!(bobs.is_none(), "Bob must not see Alice's version");
}

#[sqlx::test(migrations = "./migrations")]
async fn case_version_list_for_picker_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_story = insert_story(&pool, &users.alice, COMPETENCY).await;
    let _ = story_version_store::insert_with_next_n(
        &pool,
        &users.alice.id,
        &alice_story.id,
        &StoryBody::default(),
        None,
    )
    .await
    .expect("insert");

    let bobs = story_version_store::list_for_picker(&pool, &users.bob.id, &alice_story.id)
        .await
        .expect("picker");
    assert!(bobs.is_empty(), "Bob must see no versions on Alice's story");
}

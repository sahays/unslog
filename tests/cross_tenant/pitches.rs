//! Cross-tenant isolation for `services::pitch_store` (catalog +
//! per-user state) and `services::pitch_version_store`.
//!
//! The `pitches` catalog is shared globally — both Alice and Bob see
//! the same canonical rows. State (`pitch_user_state`) and versions
//! are strict per-owner.

use chrono::Utc;
use sqlx::PgPool;

use unslog::models::{ChatRole, ChatTurn, Pitch, PitchStatus};
use unslog::services::{pitch_store, pitch_version_store};

use crate::cross_tenant::common::setup_two_users;

const SLUG: &str = "tell-me-about-yourself";

/// Seed the catalog once per test database.
async fn seed_catalog(pool: &PgPool) {
    pitch_store::seed_defaults(pool).await.expect("seed");
}

fn turn(content: &str) -> ChatTurn {
    ChatTurn {
        role: ChatRole::User,
        content: content.into(),
        ts: Utc::now(),
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn case_catalog_is_shared_globally(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    seed_catalog(&pool).await;

    let alice_list = pitch_store::list_all(&pool, &users.alice.id)
        .await
        .expect("list");
    let bob_list = pitch_store::list_all(&pool, &users.bob.id)
        .await
        .expect("list");
    assert_eq!(alice_list.len(), bob_list.len());
    let alice_ids: Vec<String> = alice_list.iter().map(|p| p.id.clone()).collect();
    let bob_ids: Vec<String> = bob_list.iter().map(|p| p.id.clone()).collect();
    assert_eq!(alice_ids, bob_ids, "catalog is identical for both users");
    assert!(!alice_ids.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn case_state_isolated_per_user(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    seed_catalog(&pool).await;

    // Alice pushes a chat turn — creates her per-user state row.
    let mut alice_pitch = pitch_store::get(&pool, &users.alice.id, SLUG)
        .await
        .expect("get")
        .expect("present");
    pitch_store::push_turn(&pool, &users.alice.id, &mut alice_pitch, turn("from-alice"))
        .await
        .expect("push");
    assert_eq!(alice_pitch.chat.len(), 1);
    assert_eq!(alice_pitch.status, PitchStatus::InProgress);

    // Bob views the same slug — sees the not-started default, empty chat.
    let bob_view: Pitch = pitch_store::get(&pool, &users.bob.id, SLUG)
        .await
        .expect("get")
        .expect("present");
    assert!(
        bob_view.chat.is_empty(),
        "Bob must not see Alice's chat turns; got {:?}",
        bob_view.chat
    );
    assert_eq!(bob_view.status, PitchStatus::NotStarted);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_list_in_progress_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    seed_catalog(&pool).await;

    // Alice marks one pitch as in_progress.
    let mut alice_pitch = pitch_store::get(&pool, &users.alice.id, SLUG)
        .await
        .expect("get")
        .expect("present");
    pitch_store::push_turn(&pool, &users.alice.id, &mut alice_pitch, turn("hi"))
        .await
        .expect("push");

    let bob_inprog = pitch_store::list_in_progress(&pool, &users.bob.id)
        .await
        .expect("list");
    assert!(
        bob_inprog.is_empty(),
        "Bob's in-progress feed must not show Alice's"
    );

    let alice_inprog = pitch_store::list_in_progress(&pool, &users.alice.id)
        .await
        .expect("list");
    assert_eq!(alice_inprog.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_reopen_and_reset_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    seed_catalog(&pool).await;

    // Alice locks in a version on her pitch.
    let v = pitch_version_store::insert_with_next_n(&pool, &users.alice.id, SLUG, "short", "long")
        .await
        .expect("version");
    pitch_store::set_current_version(&pool, &users.alice.id, SLUG, &v.id)
        .await
        .expect("set version");

    // Bob tries to reopen Alice's pitch via his owner_id — affects HIS
    // state row, not Alice's. Alice's lock survives.
    pitch_store::reopen(&pool, &users.bob.id, SLUG)
        .await
        .expect("reopen");
    pitch_store::reset(&pool, &users.bob.id, SLUG)
        .await
        .expect("reset");

    let alice_view = pitch_store::get(&pool, &users.alice.id, SLUG)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(alice_view.status, PitchStatus::Locked);
    assert_eq!(
        alice_view.current_version_id.as_deref(),
        Some(v.id.as_str())
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn case_version_get_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    seed_catalog(&pool).await;
    let v = pitch_version_store::insert_with_next_n(&pool, &users.alice.id, SLUG, "s", "l")
        .await
        .expect("insert");

    let bobs = pitch_version_store::get(&pool, &users.bob.id, &v.id)
        .await
        .expect("get");
    assert!(bobs.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn case_version_list_for_picker_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    seed_catalog(&pool).await;
    let _ = pitch_version_store::insert_with_next_n(&pool, &users.alice.id, SLUG, "s", "l")
        .await
        .expect("insert");

    let bobs = pitch_version_store::list_for_picker(&pool, &users.bob.id, SLUG)
        .await
        .expect("picker");
    assert!(
        bobs.is_empty(),
        "Bob must see none of Alice's pitch versions"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn case_find_by_pitch_and_id_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    seed_catalog(&pool).await;
    let v = pitch_version_store::insert_with_next_n(&pool, &users.alice.id, SLUG, "s", "l")
        .await
        .expect("insert");

    let bobs = pitch_version_store::find_by_pitch_and_id(&pool, &users.bob.id, SLUG, &v.id)
        .await
        .expect("find");
    assert!(bobs.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn case_replace_in_place_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    seed_catalog(&pool).await;
    let v = pitch_version_store::insert_with_next_n(
        &pool,
        &users.alice.id,
        SLUG,
        "alice-short",
        "alice-long",
    )
    .await
    .expect("insert");

    pitch_version_store::replace_in_place(
        &pool,
        &users.bob.id,
        &v.id,
        SLUG,
        "bob-stole",
        "bob-stole",
    )
    .await
    .expect("ok with 0 rows affected");

    let still = pitch_version_store::get(&pool, &users.alice.id, &v.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(still.short, "alice-short");
    assert_eq!(still.long, "alice-long");
}

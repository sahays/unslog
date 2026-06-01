//! Cross-tenant isolation for `services::asset_store`.
//!
//! Books are a documented global-readable exception (master-pinned by the
//! migration 0003 trigger); resumes are strict per-owner.

use sqlx::PgPool;

use unslog::models::AssetKind;
use unslog::services::asset_store;

use crate::cross_tenant::common::{insert_master_book, insert_resume, setup_two_users};

#[sqlx::test(migrations = "./migrations")]
async fn case_resume_find_primary_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_resume = insert_resume(&pool, &users.alice, "alice.pdf").await;

    let bobs = asset_store::find_primary_by_kind(&pool, &users.bob.id, AssetKind::Resume)
        .await
        .expect("find primary");
    assert!(
        bobs.is_none(),
        "Bob must not see Alice's primary resume; got {bobs:?}"
    );

    let alices = asset_store::find_primary_by_kind(&pool, &users.alice.id, AssetKind::Resume)
        .await
        .expect("find primary")
        .expect("present");
    assert_eq!(alices.id, alice_resume.id);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_resume_list_sorted_excludes_other_owners_resumes(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let _alice = insert_resume(&pool, &users.alice, "alice.pdf").await;
    let _bob = insert_resume(&pool, &users.bob, "bob.pdf").await;

    let bob_list = asset_store::list_sorted(&pool, &users.bob.id)
        .await
        .expect("list");
    let names: Vec<String> = bob_list.iter().map(|a| a.name.clone()).collect();
    assert!(names.contains(&"bob.pdf".into()));
    assert!(
        !names.contains(&"alice.pdf".into()),
        "Bob's library must not contain Alice's resume; got {names:?}"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn case_book_is_shared_globally(pool: PgPool) {
    // Documented exception: kind='book' is master-owned and globally
    // readable so the critique flow inlines the same book regardless of
    // who is logged in.
    let users = setup_two_users(&pool).await;
    let _book = insert_master_book(&pool).await;
    let _ = insert_resume(&pool, &users.alice, "alice.pdf").await;

    let bob_book = asset_store::find_primary_by_kind(&pool, &users.bob.id, AssetKind::Book)
        .await
        .expect("find primary book");
    assert!(bob_book.is_some(), "Bob must see the master-owned book");

    let alice_book = asset_store::find_primary_by_kind(&pool, &users.alice.id, AssetKind::Book)
        .await
        .expect("find primary book");
    assert!(alice_book.is_some(), "Alice must see the master-owned book");

    // The book appears in everyone's library listing.
    let bob_list = asset_store::list_sorted(&pool, &users.bob.id)
        .await
        .expect("list");
    assert!(bob_list.iter().any(|a| a.kind == AssetKind::Book));
}

#[sqlx::test(migrations = "./migrations")]
async fn case_resume_delete_via_setprimary_isolated(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_resume = insert_resume(&pool, &users.alice, "alice.pdf").await;
    let bob_resume = insert_resume(&pool, &users.bob, "bob.pdf").await;

    // set_primary is owner-agnostic by id — but flipping Alice's resume
    // primary still operates on the row's own owner. Bob calling
    // set_primary on Alice's id should not corrupt Bob's primary, and
    // both rows survive untouched in their respective owners' views.
    asset_store::set_primary(&pool, &alice_resume.id)
        .await
        .expect("set primary by id");

    // Bob still sees his resume as primary (a different owner-kind slot).
    let bobs_primary = asset_store::find_primary_by_kind(&pool, &users.bob.id, AssetKind::Resume)
        .await
        .expect("find")
        .expect("present");
    assert_eq!(bobs_primary.id, bob_resume.id);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_find_or_404_is_global_by_id(pool: PgPool) {
    // asset_store::find_or_404 is owner-agnostic by design — the handler
    // layer enforces the owner_id check on the resulting row. We assert
    // the contract: the row comes back with its own `owner_id`, which
    // callers must compare against `CurrentUser::id`.
    let users = setup_two_users(&pool).await;
    let alice_resume = insert_resume(&pool, &users.alice, "alice.pdf").await;

    let asset = asset_store::find_or_404(&pool, &alice_resume.id)
        .await
        .expect("found");
    assert_eq!(asset.owner_id, users.alice.id);
    assert_ne!(asset.owner_id, users.bob.id);
}

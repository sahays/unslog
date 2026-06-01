//! Cross-tenant integration tests for the publish + fork flow.
//!
//! Spins up a per-test Postgres database via `#[sqlx::test]`, provisions
//! Alice and Bob through the shared `common` helpers, then drives the
//! production services (`services::sharing` + `services::company_fork`)
//! end-to-end. The contracts under test:
//!
//!   1. Only the owner can flip `is_public` (non-owner -> NotFound,
//!      row unchanged).
//!   2. Bob can fork Alice's public company: new row owned by Bob;
//!      Alice's source row is bit-equal pre/post.
//!   3. Owner cannot fork own (BadRequest).
//!   4. Non-public sources -> NotFound on fork (no existence leak).
//!   5. Forking writes a `forks` audit row.
//!   6. Two independent users can each fork the same source.
//!   7. /browse listing surfaces public companies across owners.
//!   8. company_store::find_or_404 still respects per-owner scoping
//!      after a publish — public flag is read-side only on /browse.

use sqlx::PgPool;

use unslog::error::AppError;
use unslog::services::{company_fork, company_store, questions as questions_svc, sharing};

use crate::cross_tenant::common::{insert_company, insert_question, setup_two_users};

// ── Publish toggle ───────────────────────────────────────────────────────

#[sqlx::test(migrations = "./migrations")]
async fn case_owner_publishes_company(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let co = insert_company(&pool, &users.alice, "Acme").await;

    sharing::set_company_public(&pool, &users.alice.id, &co.id, true)
        .await
        .expect("alice can publish her own company");

    let row = company_store::find_or_404(&pool, &users.alice.id, &co.id)
        .await
        .expect("alice still owns it");
    assert!(row.is_public, "publish must flip is_public to TRUE");
}

#[sqlx::test(migrations = "./migrations")]
async fn case_non_owner_publish_is_not_found(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let co = insert_company(&pool, &users.alice, "Acme").await;

    let err = sharing::set_company_public(&pool, &users.bob.id, &co.id, true)
        .await
        .expect_err("bob is not the owner");
    assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");

    let row = company_store::find_or_404(&pool, &users.alice.id, &co.id)
        .await
        .expect("alice still owns it");
    assert!(!row.is_public, "row must be unchanged after rejection");
}

#[sqlx::test(migrations = "./migrations")]
async fn case_owner_can_unpublish(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let co = insert_company(&pool, &users.alice, "Acme").await;
    sharing::set_company_public(&pool, &users.alice.id, &co.id, true)
        .await
        .unwrap();

    sharing::set_company_public(&pool, &users.alice.id, &co.id, false)
        .await
        .expect("owner can unpublish");

    let row = company_store::find_or_404(&pool, &users.alice.id, &co.id)
        .await
        .unwrap();
    assert!(!row.is_public);
}

// ── Browse listing ───────────────────────────────────────────────────────

#[sqlx::test(migrations = "./migrations")]
async fn case_browse_lists_only_public_across_owners(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_pub = insert_company(&pool, &users.alice, "AlphaPub").await;
    let _alice_priv = insert_company(&pool, &users.alice, "AlphaPriv").await;
    let bob_pub = insert_company(&pool, &users.bob, "BetaPub").await;

    sharing::set_company_public(&pool, &users.alice.id, &alice_pub.id, true)
        .await
        .unwrap();
    sharing::set_company_public(&pool, &users.bob.id, &bob_pub.id, true)
        .await
        .unwrap();

    let cards = sharing::list_public_companies(&pool).await.unwrap();
    let ids: Vec<String> = cards.iter().map(|c| c.id.clone()).collect();
    assert!(ids.contains(&alice_pub.id), "alice's public is listed");
    assert!(ids.contains(&bob_pub.id), "bob's public is listed");
    assert_eq!(ids.len(), 2, "no private rows leak: {ids:?}");
}

#[sqlx::test(migrations = "./migrations")]
async fn case_browse_attribution_carries_owner_label(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let co = insert_company(&pool, &users.alice, "Acme").await;
    sharing::set_company_public(&pool, &users.alice.id, &co.id, true)
        .await
        .unwrap();

    let cards = sharing::list_public_companies(&pool).await.unwrap();
    let card = cards.into_iter().find(|c| c.id == co.id).unwrap();
    assert_eq!(card.owner_label, users.alice.label);
    assert_eq!(card.owner_id, users.alice.id);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_browse_does_not_widen_owner_scoped_reads(pool: PgPool) {
    // Publishing surfaces a row on /browse but does NOT make
    // company_store::find_or_404(pool, bob, alice_id) succeed.
    let users = setup_two_users(&pool).await;
    let co = insert_company(&pool, &users.alice, "Acme").await;
    sharing::set_company_public(&pool, &users.alice.id, &co.id, true)
        .await
        .unwrap();

    let err = company_store::find_or_404(&pool, &users.bob.id, &co.id)
        .await
        .expect_err("bob still cannot read alice's row via the owner-scoped path");
    assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
}

// ── Fork ─────────────────────────────────────────────────────────────────

#[sqlx::test(migrations = "./migrations")]
async fn case_bob_forks_alice_public_company(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let alice_co = insert_company(&pool, &users.alice, "Acme").await;
    let q1 = insert_question(
        &pool,
        &users.alice,
        Some(alice_co.id.clone()),
        "behavioral q?",
    )
    .await;
    let q2 = insert_question(&pool, &users.alice, Some(alice_co.id.clone()), "tell me?").await;
    sharing::set_company_public(&pool, &users.alice.id, &alice_co.id, true)
        .await
        .unwrap();

    let pre_alice = company_store::find_or_404(&pool, &users.alice.id, &alice_co.id)
        .await
        .unwrap();
    let pre_alice_question_ids: std::collections::HashSet<String> =
        [q1.id.clone(), q2.id.clone()].iter().cloned().collect();

    let new_id = company_fork::fork(&pool, &alice_co.id, &users.bob.id)
        .await
        .expect("bob can fork alice's public company");

    // 1. The forked row is owned by Bob.
    let bob_co = company_store::find_or_404(&pool, &users.bob.id, &new_id)
        .await
        .expect("bob owns the fork");
    assert_eq!(bob_co.owner_id, users.bob.id);
    assert!(!bob_co.is_public, "forks start private");
    assert!(bob_co.name.ends_with("(forked)"));
    assert_eq!(bob_co.role, pre_alice.role);
    assert_eq!(bob_co.canonical_role, pre_alice.canonical_role);

    // 2. Alice's row is unchanged — same id, same flags, still public.
    let post_alice = company_store::find_or_404(&pool, &users.alice.id, &alice_co.id)
        .await
        .unwrap();
    assert_eq!(post_alice.id, pre_alice.id);
    assert_eq!(post_alice.owner_id, pre_alice.owner_id);
    assert_eq!(post_alice.name, pre_alice.name);
    assert!(post_alice.is_public);

    // 3. Questions copied with new ids; company_id rewritten.
    let bob_qs = questions_svc::list_for_company(&pool, &users.bob.id, &new_id)
        .await
        .unwrap();
    assert_eq!(bob_qs.len(), 2);
    for bq in &bob_qs {
        assert!(
            !pre_alice_question_ids.contains(&bq.id),
            "fork must mint new question ids: {} reused",
            bq.id
        );
        assert_eq!(bq.company_id.as_deref(), Some(new_id.as_str()));
        assert_eq!(bq.owner_id, users.bob.id);
    }
    let alice_qs = questions_svc::list_for_company(&pool, &users.alice.id, &alice_co.id)
        .await
        .unwrap();
    assert_eq!(alice_qs.len(), 2, "alice's question rows untouched");
}

#[sqlx::test(migrations = "./migrations")]
async fn case_owner_cannot_fork_own_company(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let co = insert_company(&pool, &users.alice, "Acme").await;
    sharing::set_company_public(&pool, &users.alice.id, &co.id, true)
        .await
        .unwrap();

    let err = company_fork::fork(&pool, &co.id, &users.alice.id)
        .await
        .expect_err("owner cannot fork their own row");
    assert!(matches!(err, AppError::BadRequest(_)), "got {err:?}");
}

#[sqlx::test(migrations = "./migrations")]
async fn case_fork_of_non_public_is_not_found(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let co = insert_company(&pool, &users.alice, "Acme").await;
    // Not published.

    let err = company_fork::fork(&pool, &co.id, &users.bob.id)
        .await
        .expect_err("non-public source must not be forkable");
    assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
}

#[sqlx::test(migrations = "./migrations")]
async fn case_fork_writes_audit_row(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let co = insert_company(&pool, &users.alice, "Acme").await;
    sharing::set_company_public(&pool, &users.alice.id, &co.id, true)
        .await
        .unwrap();

    let new_id = company_fork::fork(&pool, &co.id, &users.bob.id)
        .await
        .unwrap();

    let row = sqlx::query!(
        r#"SELECT id, source_kind, source_id, new_id, owner_id
           FROM forks WHERE new_id = $1"#,
        new_id,
    )
    .fetch_one(&pool)
    .await
    .expect("audit row exists");
    assert_eq!(row.source_kind, "company");
    assert_eq!(row.source_id, co.id);
    assert_eq!(row.owner_id, users.bob.id);
    assert!(row.id.starts_with("frk"));
}

#[sqlx::test(migrations = "./migrations")]
async fn case_two_users_independently_fork_same_source(pool: PgPool) {
    let users = setup_two_users(&pool).await;
    let carol = insert_third_user(&pool).await;
    let co = insert_company(&pool, &users.alice, "Acme").await;
    sharing::set_company_public(&pool, &users.alice.id, &co.id, true)
        .await
        .unwrap();

    let bob_new = company_fork::fork(&pool, &co.id, &users.bob.id)
        .await
        .unwrap();
    let carol_new = company_fork::fork(&pool, &co.id, &carol.id).await.unwrap();
    assert_ne!(bob_new, carol_new, "each fork mints a new id");

    let bobs = company_store::find_or_404(&pool, &users.bob.id, &bob_new)
        .await
        .unwrap();
    let carols = company_store::find_or_404(&pool, &carol.id, &carol_new)
        .await
        .unwrap();
    assert_eq!(bobs.owner_id, users.bob.id);
    assert_eq!(carols.owner_id, carol.id);
}

// Carol is the test's own helper — kept here so the shared `common.rs`
// stays at two-user shape that every other suite assumes.
async fn insert_third_user(pool: &PgPool) -> unslog::models::User {
    use chrono::Utc;
    use unslog::models::{User, UserTier};
    let id = "usrcarol0".to_string();
    let user = User {
        id: id.clone(),
        code_hash: format!("$argon2id$v=19$m=19456,t=2,p=1$xx${id}"),
        code_hint: "xxxx…xx".into(),
        label: "carol".into(),
        tier: UserTier::Pro,
        is_master: false,
        activated_at: None,
        expires_at: None,
        last_seen_at: None,
        revoked_at: None,
        invited_by: None,
        created_at: Utc::now(),
    };
    unslog::services::user_store::insert(pool, &user)
        .await
        .expect("insert carol");
    user
}

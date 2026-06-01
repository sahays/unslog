//! Cross-tenant invariants for the invite-admin surface.
//!
//! `/settings/invites/*` is gated by `require_master_middleware` at the
//! HTTP layer. The DB-side guarantees we test here are the second line
//! of defence: if a non-master ever somehow reaches the service, the
//! SQL itself still refuses to touch the master row, and the
//! `users.is_master = FALSE` clause keeps a stray request from
//! escalating its own privileges.

use sqlx::PgPool;

use unslog::error::AppError;
use unslog::models::UserTier;
use unslog::services::invite_admin::{InviteAdminDeps, MintInput, PgInviteAdminDeps};

#[sqlx::test(migrations = "./migrations")]
async fn case_mint_then_revoke_blocks_subsequent_login(pool: PgPool) {
    // End-to-end shape: master mints code; the user shows up in the
    // active-login scan; revoke; the same scan no longer returns them.
    let deps = PgInviteAdminDeps { pool: pool.clone() };
    let minted = deps
        .mint(MintInput {
            label: "alice".into(),
            tier: UserTier::Pro,
            days: 30,
        })
        .await
        .unwrap();

    let before = unslog::services::user_store::list_active_for_login(&pool)
        .await
        .unwrap();
    let alice_present = before.iter().any(|u| u.id == minted.id);
    assert!(alice_present, "freshly minted user is in the login scan");

    deps.revoke(&minted.id).await.unwrap();

    let after = unslog::services::user_store::list_active_for_login(&pool)
        .await
        .unwrap();
    let alice_still_present = after.iter().any(|u| u.id == minted.id);
    assert!(
        !alice_still_present,
        "revoked user must not appear in the login scan"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn case_extend_never_targets_master_row(pool: PgPool) {
    // Even with the literal master id, extend refuses to move the
    // master's expires_at — `is_master = FALSE` in the WHERE clause
    // makes the row invisible to the UPDATE.
    let deps = PgInviteAdminDeps { pool };
    let err = deps
        .extend(unslog::services::master_seed::MASTER_ID, 30)
        .await
        .expect_err("extend on master must NotFound");
    assert!(matches!(err, AppError::NotFound(_)));
}

#[sqlx::test(migrations = "./migrations")]
async fn case_revoke_never_targets_master_row(pool: PgPool) {
    let deps = PgInviteAdminDeps { pool };
    let err = deps
        .revoke(unslog::services::master_seed::MASTER_ID)
        .await
        .expect_err("revoke on master must NotFound");
    assert!(matches!(err, AppError::NotFound(_)));
}

#[sqlx::test(migrations = "./migrations")]
async fn case_list_invitees_excludes_master(pool: PgPool) {
    // Migration 0003 seeds the master row; the admin listing must not
    // return it (admins see invitees only on this surface).
    let deps = PgInviteAdminDeps { pool: pool.clone() };
    let _ = deps
        .mint(MintInput {
            label: "alice".into(),
            tier: UserTier::Pro,
            days: 30,
        })
        .await
        .unwrap();

    let rows = deps.list_invitees().await.unwrap();
    assert!(
        rows.iter().all(|r| r.id != "usrmaster"),
        "master row must be hidden from /settings/invites"
    );
    assert_eq!(rows.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_revoking_one_invite_does_not_affect_others(pool: PgPool) {
    // Per-row scoping: revoking Alice's invite leaves Bob's untouched.
    let deps = PgInviteAdminDeps { pool: pool.clone() };
    let alice = deps
        .mint(MintInput {
            label: "alice".into(),
            tier: UserTier::Pro,
            days: 30,
        })
        .await
        .unwrap();
    let bob = deps
        .mint(MintInput {
            label: "bob".into(),
            tier: UserTier::ProMax,
            days: 60,
        })
        .await
        .unwrap();

    deps.revoke(&alice.id).await.unwrap();

    let active = unslog::services::user_store::list_active_for_login(&pool)
        .await
        .unwrap();
    assert!(active.iter().any(|u| u.id == bob.id));
    assert!(!active.iter().any(|u| u.id == alice.id));
}

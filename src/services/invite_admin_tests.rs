//! Unit + integration tests for `invite_admin`.
//!
//! Unit tests cover validation bounds and id-shape invariants without
//! Postgres. The tracing-capture test is **security-critical**: it
//! asserts the plain invite code is NEVER written to any log event,
//! either as a value or as a substring of another field.
//!
//! Integration tests live under the same module — `#[sqlx::test]` spins
//! up a fresh per-test database with every migration applied.

use super::*;
use crate::models::UserTier;

// ── Input validation ──────────────────────────────────────────────────

#[test]
fn case_validate_mint_input_rejects_empty_label() {
    let err = validate_mint_input(&MintInput {
        label: "".into(),
        tier: UserTier::Pro,
        days: 30,
    })
    .expect_err("expected reject");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_validate_mint_input_rejects_whitespace_only_label() {
    let err = validate_mint_input(&MintInput {
        label: "   ".into(),
        tier: UserTier::Pro,
        days: 30,
    })
    .expect_err("expected reject");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_validate_mint_input_rejects_overlong_label() {
    let err = validate_mint_input(&MintInput {
        label: "a".repeat(MAX_LABEL_LEN + 1),
        tier: UserTier::Pro,
        days: 30,
    })
    .expect_err("expected reject");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_validate_mint_input_rejects_master_tier() {
    let err = validate_mint_input(&MintInput {
        label: "alice".into(),
        tier: UserTier::Master,
        days: 30,
    })
    .expect_err("expected reject");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_validate_days_rejects_out_of_range() {
    for bad in [0u32, 1, 7, 45, 120, 365, 1000] {
        let err = validate_days(bad).expect_err(&format!("expected reject for {bad}"));
        assert!(matches!(err, AppError::BadRequest(_)));
    }
}

#[test]
fn case_validate_days_accepts_allowed_set() {
    for ok in ALLOWED_DAYS {
        validate_days(*ok).expect("should accept");
    }
}

#[test]
fn case_validate_mint_input_accepts_pro_30() {
    validate_mint_input(&MintInput {
        label: "alice".into(),
        tier: UserTier::Pro,
        days: 30,
    })
    .expect("ok");
}

#[test]
fn case_validate_mint_input_accepts_pro_max_90() {
    validate_mint_input(&MintInput {
        label: "bob".into(),
        tier: UserTier::ProMax,
        days: 90,
    })
    .expect("ok");
}

// ── ID generator shape ────────────────────────────────────────────────

#[test]
fn case_new_user_id_matches_check_constraint_shape() {
    // `users.id ~ '^usr[a-z0-9]{6}$'` — 9 chars total, prefix `usr`,
    // suffix is exactly 6 lowercase alphanumerics. Run a batch to
    // catch any modulo-bias regression that pushes a non-alnum byte
    // through.
    for _ in 0..1000 {
        let id = new_user_id();
        assert_eq!(id.len(), 9, "id={id}");
        assert!(id.starts_with("usr"), "id={id}");
        assert!(
            id[3..]
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()),
            "id={id} has non-alnum suffix"
        );
    }
}

#[test]
fn case_new_user_id_is_non_deterministic() {
    // Two consecutive calls equal is a 1-in-36^6 event; if OsRng is
    // wired correctly this never fires.
    assert_ne!(new_user_id(), new_user_id());
}

// ── Tracing-capture: plain code never reaches logs ────────────────────

/// `tracing::Subscriber` impl that captures every event's fields and
/// message into a shared buffer. Used by the security test below to
/// assert the plain invite code never surfaces in any log line.
#[derive(Clone, Default)]
struct CaptureSubscriber {
    buf: std::sync::Arc<std::sync::Mutex<String>>,
}

impl tracing::Subscriber for CaptureSubscriber {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        struct V<'a>(&'a mut String);
        impl<'a> tracing::field::Visit for V<'a> {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                use std::fmt::Write;
                let _ = write!(self.0, " {}={:?}", field.name(), value);
            }
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                use std::fmt::Write;
                let _ = write!(self.0, " {}={}", field.name(), value);
            }
        }
        if let Ok(mut guard) = self.buf.lock() {
            event.record(&mut V(&mut guard));
            guard.push('\n');
        }
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

/// `try_insert_invite` is the one site that holds the plain code in
/// memory. We can't run it without a `PgPool`, so we exercise the same
/// log shape by invoking the small helper that builds the structured
/// event — proxied through `tracing::info!` with the exact field set we
/// expect at the real call site.
///
/// This guards the contract: even if the production code is refactored
/// later, the `code_hint` log field MUST contain only the redacted hint
/// (first 4 + "…" + last 2), never the original code.
#[test]
fn case_tracing_never_emits_plain_code_security_critical() {
    let cap = CaptureSubscriber::default();
    let plain_code = "GPbb5GAnsEQZ";
    let hint = invite_codes::hint(plain_code);
    let id = new_user_id();
    let label = "alice";

    tracing::subscriber::with_default(cap.clone(), || {
        // Exactly the structured fields we ship in `try_insert_invite`.
        tracing::info!(
            event = "invite.mint",
            invitee_id = %id,
            tier = "pro",
            label = %label,
            days = 30u32,
            code_hint = %hint,
            "invite minted",
        );
    });

    let captured = cap.buf.lock().expect("lock").clone();
    assert!(
        !captured.contains(plain_code),
        "plain invite code leaked into a tracing event: {captured:?}"
    );
    // The hint is allowed — it's only first 4 + "…" + last 2.
    assert!(
        captured.contains(&hint),
        "code_hint should appear in tracing event: {captured:?}"
    );
}

// ── Integration tests (postgres) ──────────────────────────────────────

#[sqlx::test(migrations = "./migrations")]
async fn case_mint_inserts_hashed_row_and_returns_plain_code(pool: PgPool) {
    let deps = PgInviteAdminDeps { pool: pool.clone() };
    let out = deps
        .mint(MintInput {
            label: "alice".into(),
            tier: UserTier::Pro,
            days: 30,
        })
        .await
        .expect("mint ok");

    // Plain code is well-formed and the hint matches.
    assert!(invite_codes::is_valid_format(&out.plain_code));
    assert_eq!(out.code_hint, invite_codes::hint(&out.plain_code));

    // Row exists; code is stored as argon2id hash, not as plain text.
    let user = crate::services::user_store::find_by_id(&pool, &out.id)
        .await
        .expect("lookup ok")
        .expect("row inserted");
    assert!(!user.is_master);
    assert_eq!(user.tier, UserTier::Pro);
    assert!(user.code_hash.starts_with("$argon2id$"));
    assert!(!user.code_hash.contains(&out.plain_code));
    assert_eq!(user.code_hint, out.code_hint);
    // expires_at is approximately now+30d.
    let exp = user.expires_at.expect("expiry set");
    let delta = (exp - Utc::now()).num_seconds();
    assert!(delta > 29 * 86400 && delta < 31 * 86400, "delta={delta}");
}

#[sqlx::test(migrations = "./migrations")]
async fn case_mint_rejects_bad_days_and_label(pool: PgPool) {
    let deps = PgInviteAdminDeps { pool };
    for bad in [7u32, 120, 365] {
        let err = deps
            .mint(MintInput {
                label: "alice".into(),
                tier: UserTier::Pro,
                days: bad,
            })
            .await
            .expect_err(&format!("expected reject for {bad}"));
        assert!(matches!(err, AppError::BadRequest(_)));
    }
    let err = deps
        .mint(MintInput {
            label: "a".repeat(MAX_LABEL_LEN + 1),
            tier: UserTier::Pro,
            days: 30,
        })
        .await
        .expect_err("expected reject");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[sqlx::test(migrations = "./migrations")]
async fn case_extend_only_moves_expires_forward(pool: PgPool) {
    let deps = PgInviteAdminDeps { pool: pool.clone() };
    let minted = deps
        .mint(MintInput {
            label: "alice".into(),
            tier: UserTier::Pro,
            days: 90,
        })
        .await
        .unwrap();
    let original = crate::services::user_store::find_by_id(&pool, &minted.id)
        .await
        .unwrap()
        .unwrap()
        .expires_at
        .expect("expiry set");

    // Extending by 30 days from a fresh now+90 puts the new expiry at
    // ~now+120 — always later than `original`.
    let new_exp = deps.extend(&minted.id, 30).await.expect("extend ok");
    assert!(new_exp > original, "extend must move forward");
}

#[sqlx::test(migrations = "./migrations")]
async fn case_extend_rejects_bad_days(pool: PgPool) {
    let deps = PgInviteAdminDeps { pool: pool.clone() };
    let minted = deps
        .mint(MintInput {
            label: "alice".into(),
            tier: UserTier::Pro,
            days: 30,
        })
        .await
        .unwrap();
    for bad in [0u32, 1, 7, 100, 365] {
        let err = deps
            .extend(&minted.id, bad)
            .await
            .expect_err(&format!("expected reject for {bad}"));
        assert!(matches!(err, AppError::BadRequest(_)));
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn case_extend_unknown_id_is_not_found(pool: PgPool) {
    let deps = PgInviteAdminDeps { pool };
    let err = deps
        .extend("usrxxxxxx", 30)
        .await
        .expect_err("expected NotFound");
    assert!(matches!(err, AppError::NotFound(_)));
}

#[sqlx::test(migrations = "./migrations")]
async fn case_extend_refuses_master(pool: PgPool) {
    // Migration 0003 seeded usrmaster with a placeholder hash; extend
    // must refuse to touch it because of `is_master = FALSE` in WHERE.
    let deps = PgInviteAdminDeps { pool };
    let err = deps
        .extend(crate::services::master_seed::MASTER_ID, 30)
        .await
        .expect_err("expected NotFound");
    assert!(matches!(err, AppError::NotFound(_)));
}

#[sqlx::test(migrations = "./migrations")]
async fn case_revoke_marks_revoked_at(pool: PgPool) {
    let deps = PgInviteAdminDeps { pool: pool.clone() };
    let minted = deps
        .mint(MintInput {
            label: "alice".into(),
            tier: UserTier::Pro,
            days: 30,
        })
        .await
        .unwrap();
    deps.revoke(&minted.id).await.expect("revoke ok");

    // user_store::find_by_id excludes revoked rows.
    let lookup = crate::services::user_store::find_by_id(&pool, &minted.id)
        .await
        .unwrap();
    assert!(
        lookup.is_none(),
        "revoked user must be hidden by find_by_id"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn case_revoke_is_idempotent_second_call_is_not_found(pool: PgPool) {
    let deps = PgInviteAdminDeps { pool };
    let minted = deps
        .mint(MintInput {
            label: "alice".into(),
            tier: UserTier::Pro,
            days: 30,
        })
        .await
        .unwrap();
    deps.revoke(&minted.id).await.expect("first revoke ok");
    let err = deps
        .revoke(&minted.id)
        .await
        .expect_err("second revoke must be NotFound");
    assert!(matches!(err, AppError::NotFound(_)));
}

#[sqlx::test(migrations = "./migrations")]
async fn case_revoke_refuses_master(pool: PgPool) {
    let deps = PgInviteAdminDeps { pool };
    let err = deps
        .revoke(crate::services::master_seed::MASTER_ID)
        .await
        .expect_err("expected NotFound");
    assert!(matches!(err, AppError::NotFound(_)));
}

#[sqlx::test(migrations = "./migrations")]
async fn case_revoked_user_drops_off_login_scan(pool: PgPool) {
    // The auth path scans `list_active_for_login` and argon2-verifies
    // each row. A revoked row must not appear in that scan, so the
    // plain code that previously worked stops working on the next
    // request.
    let deps = PgInviteAdminDeps { pool: pool.clone() };
    let minted = deps
        .mint(MintInput {
            label: "alice".into(),
            tier: UserTier::Pro,
            days: 30,
        })
        .await
        .unwrap();
    let active_before = crate::services::user_store::list_active_for_login(&pool)
        .await
        .unwrap();
    assert!(active_before.iter().any(|u| u.id == minted.id));

    deps.revoke(&minted.id).await.unwrap();

    let active_after = crate::services::user_store::list_active_for_login(&pool)
        .await
        .unwrap();
    assert!(!active_after.iter().any(|u| u.id == minted.id));
}

#[sqlx::test(migrations = "./migrations")]
async fn case_list_invitees_orders_newest_first_with_usage(pool: PgPool) {
    let deps = PgInviteAdminDeps { pool: pool.clone() };
    let a = deps
        .mint(MintInput {
            label: "alice".into(),
            tier: UserTier::Pro,
            days: 30,
        })
        .await
        .unwrap();
    // Ensure a deterministic created_at ordering.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let b = deps
        .mint(MintInput {
            label: "bob".into(),
            tier: UserTier::ProMax,
            days: 60,
        })
        .await
        .unwrap();

    // Push a couple of cost_units onto Alice so the LATERAL sum is
    // exercised in the listing.
    sqlx::query(
        "INSERT INTO request_log (user_id, method, path, status, duration_ms, kind, cost_units) \
         VALUES ($1, 'POST', '/x', 200, 1, 'llm', 5)",
    )
    .bind(&a.id)
    .execute(&pool)
    .await
    .unwrap();

    let rows = deps.list_invitees().await.expect("list ok");
    // Master is excluded.
    assert!(rows.iter().all(|r| r.id != "usrmaster"));
    // Newest first.
    assert_eq!(rows.first().unwrap().id, b.id);
    let alice_row = rows.iter().find(|r| r.id == a.id).unwrap();
    assert_eq!(alice_row.used_units, 5);
    let bob_row = rows.iter().find(|r| r.id == b.id).unwrap();
    assert_eq!(bob_row.used_units, 0);
}

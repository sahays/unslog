use super::*;
use crate::services::cookies;
use chrono::{Duration, Utc};
use mockall::predicate;

const KEY: &[u8] = b"0123456789abcdef0123456789abcdef";

fn fixture_active() -> User {
    User {
        id: "usrabc123".into(),
        code_hash: "$argon2id$v=19$m=19456,t=2,p=1$xyz$pqr".into(),
        code_hint: "GPbb…QZ".into(),
        label: "guest 1".into(),
        tier: UserTier::Pro,
        is_master: false,
        activated_at: Some(Utc::now()),
        expires_at: None,
        last_seen_at: None,
        revoked_at: None,
        invited_by: Some("usrmaster".into()),
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn case_valid_cookie_and_active_user_returns_current_user() {
    let user = fixture_active();
    let token = cookies::sign(&user.id, Utc::now().timestamp(), KEY);

    let mut deps = MockAuthDeps::new();
    deps.expect_find_by_id()
        .with(predicate::eq("usrabc123"))
        .returning(move |_| Ok(Some(fixture_active())));
    deps.expect_touch_last_seen()
        .with(predicate::eq("usrabc123"))
        .returning(|_| Ok(()));

    let cu = authenticate(&token, KEY, 3600, &deps).await.expect("ok");
    assert_eq!(cu.id, "usrabc123");
    assert_eq!(cu.label, "guest 1");
    assert!(!cu.is_master);
}

#[tokio::test]
async fn case_revoked_user_returns_unauthenticated() {
    let user = fixture_active();
    let token = cookies::sign(&user.id, Utc::now().timestamp(), KEY);

    let mut deps = MockAuthDeps::new();
    deps.expect_find_by_id().returning(|_| {
        let mut u = fixture_active();
        u.revoked_at = Some(Utc::now());
        Ok(Some(u))
    });
    // touch_last_seen must NOT be called for revoked users.
    deps.expect_touch_last_seen().times(0);

    let err = authenticate(&token, KEY, 3600, &deps)
        .await
        .expect_err("err");
    assert!(matches!(err, AppError::Unauthenticated));
}

#[tokio::test]
async fn case_expired_user_returns_unauthenticated() {
    let token = cookies::sign("usrabc123", Utc::now().timestamp(), KEY);

    let mut deps = MockAuthDeps::new();
    deps.expect_find_by_id().returning(|_| {
        let mut u = fixture_active();
        u.expires_at = Some(Utc::now() - Duration::hours(1));
        Ok(Some(u))
    });
    deps.expect_touch_last_seen().times(0);

    let err = authenticate(&token, KEY, 3600, &deps)
        .await
        .expect_err("err");
    assert!(matches!(err, AppError::Unauthenticated));
}

#[tokio::test]
async fn case_unknown_user_returns_unauthenticated() {
    let token = cookies::sign("usrabc123", Utc::now().timestamp(), KEY);

    let mut deps = MockAuthDeps::new();
    deps.expect_find_by_id().returning(|_| Ok(None));
    deps.expect_touch_last_seen().times(0);

    let err = authenticate(&token, KEY, 3600, &deps)
        .await
        .expect_err("err");
    assert!(matches!(err, AppError::Unauthenticated));
}

#[tokio::test]
async fn case_tampered_cookie_returns_unauthenticated_without_db_lookup() {
    let mut token = cookies::sign("usrabc123", Utc::now().timestamp(), KEY);
    // Flip first byte — invalidates the HMAC.
    let byte = token.as_bytes()[0];
    let swap = if byte == b'A' { b'B' } else { b'A' };
    unsafe {
        token.as_bytes_mut()[0] = swap;
    }

    let mut deps = MockAuthDeps::new();
    deps.expect_find_by_id().times(0);
    deps.expect_touch_last_seen().times(0);

    let err = authenticate(&token, KEY, 3600, &deps)
        .await
        .expect_err("err");
    assert!(matches!(err, AppError::Unauthenticated));
}

#[tokio::test]
async fn case_touch_last_seen_failure_does_not_break_request() {
    let token = cookies::sign("usrabc123", Utc::now().timestamp(), KEY);

    let mut deps = MockAuthDeps::new();
    deps.expect_find_by_id()
        .returning(|_| Ok(Some(fixture_active())));
    deps.expect_touch_last_seen().returning(|_| {
        Err(AppError::Other(anyhow::anyhow!(
            "simulated db failure on touch"
        )))
    });

    let cu = authenticate(&token, KEY, 3600, &deps).await.expect("ok");
    assert_eq!(cu.id, "usrabc123");
}

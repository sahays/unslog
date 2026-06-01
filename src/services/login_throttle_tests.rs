//! Logic-only tests for the throttle helpers. The trait surface is mocked
//! by Phase 1.2 callers; integration tests against a live `login_attempts`
//! table are deferred until the `/login` handler lands (next sub-step).

use super::*;
use mockall::predicate;
use std::net::{IpAddr, Ipv4Addr};

#[test]
fn case_clip_prefix_truncates_to_4() {
    assert_eq!(clip_prefix("GPbb5GAnsEQZ"), "GPbb");
    assert_eq!(clip_prefix("abc"), "abc");
    assert_eq!(clip_prefix(""), "");
    // Multi-byte safety — `chars().take(4)` walks scalars, not bytes.
    assert_eq!(clip_prefix("éèêë more"), "éèêë");
}

#[tokio::test]
async fn case_check_returns_ok_under_cap() {
    let mut t = MockLoginThrottle::new();
    let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    t.expect_check()
        .with(
            predicate::eq(ip),
            predicate::eq(5u32),
            predicate::eq(300u64),
        )
        .times(1)
        .returning(|_, _, _| Ok(()));
    t.check(&ip, 5, 300).await.expect("ok");
}

#[tokio::test]
async fn case_check_returns_rate_limited_at_cap() {
    let mut t = MockLoginThrottle::new();
    let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    t.expect_check()
        .times(1)
        .returning(|_, _, _| Err(AppError::RateLimited { used: 5, cap: 5 }));
    let err = t.check(&ip, 5, 300).await.expect_err("err");
    assert!(matches!(err, AppError::RateLimited { used: 5, cap: 5 }));
}

#[tokio::test]
async fn case_record_failure_passes_prefix_through() {
    let mut t = MockLoginThrottle::new();
    let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    t.expect_record_failure()
        .with(predicate::eq(ip), predicate::eq("GPbb"))
        .times(1)
        .returning(|_, _| Ok(()));
    t.record_failure(&ip, "GPbb").await.expect("ok");
}

#[tokio::test]
async fn case_record_success_passes_prefix_through() {
    let mut t = MockLoginThrottle::new();
    let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    t.expect_record_success()
        .with(predicate::eq(ip), predicate::eq("GPbb"))
        .times(1)
        .returning(|_, _| Ok(()));
    t.record_success(&ip, "GPbb").await.expect("ok");
}

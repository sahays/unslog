use super::*;
use crate::models::UserTier;
use crate::services::auth::CurrentUser;

fn pro_user() -> CurrentUser {
    CurrentUser {
        id: "usrpro001".into(),
        label: "pro".into(),
        tier: UserTier::Pro,
        is_master: false,
    }
}

fn master_user() -> CurrentUser {
    CurrentUser {
        id: "usrmaster".into(),
        label: "master".into(),
        tier: UserTier::Master,
        is_master: true,
    }
}

#[tokio::test]
async fn case_under_cap_returns_usage_window() {
    let mut deps = MockMeteringDeps::new();
    deps.expect_tier_cap_for().returning(|_| Some(100));
    deps.expect_sum_units_window().returning(|_, _| Ok(40));
    let win = check_or_block(&deps, &pro_user(), 5).await.unwrap();
    assert_eq!(win.used, 40);
    assert_eq!(win.cap, 100);
}

#[tokio::test]
async fn case_at_cap_returns_rate_limited() {
    let mut deps = MockMeteringDeps::new();
    deps.expect_tier_cap_for().returning(|_| Some(100));
    deps.expect_sum_units_window().returning(|_, _| Ok(99));
    let err = check_or_block(&deps, &pro_user(), 5).await.unwrap_err();
    match err {
        AppError::RateLimited { used, cap } => {
            assert_eq!(used, 99);
            assert_eq!(cap, 100);
        }
        other => panic!("expected RateLimited, got {other:?}"),
    }
}

#[tokio::test]
async fn case_master_never_blocked_even_at_high_usage() {
    let mut deps = MockMeteringDeps::new();
    deps.expect_tier_cap_for().returning(|_| None);
    deps.expect_sum_units_window().returning(|_, _| Ok(10_000));
    let win = check_or_block(&deps, &master_user(), 5).await.unwrap();
    assert_eq!(win.cap, u64::MAX);
    assert_eq!(win.used, 10_000);
}

#[tokio::test]
async fn case_current_window_degrades_on_query_failure() {
    let mut deps = MockMeteringDeps::new();
    deps.expect_tier_cap_for().returning(|_| Some(200));
    deps.expect_sum_units_window()
        .returning(|_, _| Err(AppError::Other(anyhow::anyhow!("boom"))));
    let win = current_window(&deps, &pro_user()).await;
    assert_eq!(win.used, 0);
    assert_eq!(win.cap, 200);
}

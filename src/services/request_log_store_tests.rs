//! Integration tests for `PgMeteringDeps`. Uses `#[sqlx::test]` so each
//! test gets a fresh per-test database with every migration applied —
//! same pattern as the cross-tenant isolation suite.

use super::*;
use chrono::Utc;

fn pro_user(id: &str) -> CurrentUser {
    CurrentUser {
        id: id.to_string(),
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

fn test_config() -> AppConfig {
    AppConfig {
        host: "127.0.0.1".into(),
        port: 0,
        database_url: String::new(),
        openrouter_api_key: String::new(),
        data_dir: String::new(),
        log_format: crate::config::LogFormat::Compact,
        log_dir: String::new(),
        referer: None,
        session_cookie_name: "__Host-session".into(),
        csrf_cookie_name: "__Host-csrf".into(),
        login_throttle_max_attempts: 5,
        login_throttle_window_secs: 300,
        session_max_age_secs: 2_592_000,
        pro_request_cap_daily: 100,
        pro_max_request_cap_daily: 500,
        dev_insecure: false,
        master_invite_code: None,
        master_user_label: "master".into(),
    }
}

async fn insert_user(pool: &PgPool, id: &str, is_master: bool) {
    sqlx::query(
        "INSERT INTO users (id, code_hash, code_hint, label, tier, is_master, created_at) \
         VALUES ($1, $2, 'xx…xx', $3, $4, $5, NOW())",
    )
    .bind(id)
    .bind(format!("hash-{id}"))
    .bind(id)
    .bind(if is_master { "master" } else { "pro" })
    .bind(is_master)
    .execute(pool)
    .await
    .expect("insert user");
}

#[sqlx::test(migrations = "./migrations")]
async fn case_append_and_sum_round_trip(pool: PgPool) {
    insert_user(&pool, "usralice0", false).await;
    let cfg = test_config();
    let deps = PgMeteringDeps {
        pool: &pool,
        config: &cfg,
    };
    deps.append(AppendRow {
        user_id: "usralice0".into(),
        method: "POST".into(),
        path: "/sessions/x/answers".into(),
        status: 200,
        duration_ms: 1234,
        kind: "llm".into(),
        cost_units: 5,
    })
    .await
    .unwrap();
    deps.append(AppendRow {
        user_id: "usralice0".into(),
        method: "POST".into(),
        path: "/sessions/x/next-question".into(),
        status: 200,
        duration_ms: 50,
        kind: "llm".into(),
        cost_units: 1,
    })
    .await
    .unwrap();
    let used = deps
        .sum_units_window("usralice0", Utc::now() - chrono::Duration::hours(24))
        .await
        .unwrap();
    assert_eq!(used, 6);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_sum_isolated_per_user(pool: PgPool) {
    insert_user(&pool, "usralice0", false).await;
    insert_user(&pool, "usrbob001", false).await;
    let cfg = test_config();
    let deps = PgMeteringDeps {
        pool: &pool,
        config: &cfg,
    };
    deps.append(AppendRow {
        user_id: "usralice0".into(),
        method: "POST".into(),
        path: "/x".into(),
        status: 200,
        duration_ms: 1,
        kind: "llm".into(),
        cost_units: 10,
    })
    .await
    .unwrap();
    deps.append(AppendRow {
        user_id: "usrbob001".into(),
        method: "POST".into(),
        path: "/x".into(),
        status: 200,
        duration_ms: 1,
        kind: "llm".into(),
        cost_units: 7,
    })
    .await
    .unwrap();
    let alice = deps
        .sum_units_window("usralice0", Utc::now() - chrono::Duration::hours(24))
        .await
        .unwrap();
    let bob = deps
        .sum_units_window("usrbob001", Utc::now() - chrono::Duration::hours(24))
        .await
        .unwrap();
    assert_eq!(alice, 10);
    assert_eq!(bob, 7);
}

#[sqlx::test(migrations = "./migrations")]
async fn case_tier_cap_for_master_is_none(pool: PgPool) {
    let cfg = test_config();
    let deps = PgMeteringDeps {
        pool: &pool,
        config: &cfg,
    };
    assert_eq!(deps.tier_cap_for(&master_user()), None);
    assert_eq!(deps.tier_cap_for(&pro_user("usralice0")), Some(100));
}

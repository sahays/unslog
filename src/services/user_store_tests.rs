use super::*;
use crate::models::{User, UserTier};
use chrono::Utc;

fn fixture_master() -> User {
    User {
        id: "usrmaster".into(),
        code_hash: "$argon2id$v=19$m=19456,t=2,p=1$abc$def".into(),
        code_hint: "GPbb…QZ".into(),
        label: "master".into(),
        tier: UserTier::Master,
        is_master: true,
        activated_at: None,
        expires_at: None,
        last_seen_at: None,
        revoked_at: None,
        invited_by: None,
        created_at: Utc::now(),
    }
}

fn fixture_pro() -> User {
    User {
        id: "usrabc123".into(),
        code_hash: "$argon2id$v=19$m=19456,t=2,p=1$xyz$pqr".into(),
        code_hint: "GPbb…QZ".into(),
        label: "guest 1".into(),
        tier: UserTier::Pro,
        is_master: false,
        activated_at: None,
        expires_at: None,
        last_seen_at: None,
        revoked_at: None,
        invited_by: Some("usrmaster".into()),
        created_at: Utc::now(),
    }
}

#[test]
fn case_validate_master_must_be_tier_master() {
    let mut u = fixture_master();
    u.tier = UserTier::Pro;
    let err = validate_invariants(&u).expect_err("err");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_validate_master_must_not_have_expiry() {
    let mut u = fixture_master();
    u.expires_at = Some(Utc::now());
    let err = validate_invariants(&u).expect_err("err");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_validate_non_master_must_not_be_tier_master() {
    let mut u = fixture_pro();
    u.tier = UserTier::Master;
    let err = validate_invariants(&u).expect_err("err");
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[test]
fn case_validate_accepts_canonical_master_and_pro() {
    validate_invariants(&fixture_master()).expect("master ok");
    validate_invariants(&fixture_pro()).expect("pro ok");
}

#[test]
fn case_is_active_rejects_revoked() {
    let mut u = fixture_pro();
    u.revoked_at = Some(Utc::now());
    assert!(!u.is_active(Utc::now()));
}

#[test]
fn case_is_active_rejects_expired() {
    let mut u = fixture_pro();
    u.expires_at = Some(Utc::now() - chrono::Duration::hours(1));
    assert!(!u.is_active(Utc::now()));
}

#[test]
fn case_is_active_accepts_future_expiry() {
    let mut u = fixture_pro();
    u.expires_at = Some(Utc::now() + chrono::Duration::hours(1));
    assert!(u.is_active(Utc::now()));
}

#[test]
fn case_is_active_master_never_expires() {
    let u = fixture_master();
    assert!(u.is_active(Utc::now()));
}

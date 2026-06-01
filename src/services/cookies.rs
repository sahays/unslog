//! HMAC-SHA256 signed session cookie value.
//!
//! Token format: `base64url({user_id}.{issued_at}).base64url(HMAC-SHA256)`.
//! The MAC covers the inner `{user_id}.{issued_at}` payload as raw bytes,
//! so changing either field after issuance fails the verify step.
//!
//! Verification uses `subtle::ConstantTimeEq` — never a `==` on the MAC
//! bytes — and also rejects tokens older than `max_age_secs` against the
//! caller-supplied `now` (so callers can pin the clock in tests).

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::error::AppError;

type HmacSha256 = Hmac<Sha256>;

/// Sign `(user_id, issued_at)` with `key` and return the wire-format token.
/// `issued_at` is Unix epoch seconds (i64) so the same encoding survives
/// pre-1970 test fixtures without panicking.
pub fn sign(user_id: &str, issued_at: i64, key: &[u8]) -> String {
    let payload = format!("{user_id}.{issued_at}");
    let mac = mac_bytes(key, payload.as_bytes());
    let payload_b64 = URL_SAFE_NO_PAD.encode(payload.as_bytes());
    let mac_b64 = URL_SAFE_NO_PAD.encode(mac);
    format!("{payload_b64}.{mac_b64}")
}

/// Verify `token` against `key` and an age budget. Returns the embedded
/// `user_id` on success, [`AppError::Unauthenticated`] on any failure.
pub fn verify(token: &str, key: &[u8], max_age_secs: i64, now: i64) -> Result<String, AppError> {
    let (payload_b64, mac_b64) = token.split_once('.').ok_or(AppError::Unauthenticated)?;
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| AppError::Unauthenticated)?;
    let mac_bytes_recv = URL_SAFE_NO_PAD
        .decode(mac_b64)
        .map_err(|_| AppError::Unauthenticated)?;
    let mac_expected = mac_bytes(key, &payload_bytes);
    if mac_bytes_recv.ct_eq(&mac_expected).unwrap_u8() != 1 {
        return Err(AppError::Unauthenticated);
    }
    let payload = std::str::from_utf8(&payload_bytes).map_err(|_| AppError::Unauthenticated)?;
    let (user_id, issued_at_str) = payload.rsplit_once('.').ok_or(AppError::Unauthenticated)?;
    let issued_at: i64 = issued_at_str
        .parse()
        .map_err(|_| AppError::Unauthenticated)?;
    if now.saturating_sub(issued_at) > max_age_secs {
        return Err(AppError::Unauthenticated);
    }
    Ok(user_id.to_string())
}

fn mac_bytes(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &[u8] = b"0123456789abcdef0123456789abcdef";

    #[test]
    fn case_round_trip_recovers_user_id() {
        let token = sign("usrabc123", 1_700_000_000, KEY);
        let id = verify(&token, KEY, 3600, 1_700_000_010).expect("ok");
        assert_eq!(id, "usrabc123");
    }

    #[test]
    fn case_tampered_payload_is_rejected() {
        let mut token = sign("usrabc123", 1_700_000_000, KEY);
        // Flip a char in the payload (first segment).
        let mid = token.find('.').unwrap();
        let byte = token.as_bytes()[0];
        // Swap to a different valid base64-url char.
        let swap = if byte == b'A' { b'B' } else { b'A' };
        unsafe {
            token.as_bytes_mut()[0] = swap;
        }
        // Re-touch to silence borrow checker; mid is still valid since
        // length didn't change.
        let _ = mid;
        let err = verify(&token, KEY, 3600, 1_700_000_010).expect_err("err");
        assert!(matches!(err, AppError::Unauthenticated));
    }

    #[test]
    fn case_expired_token_is_rejected() {
        let token = sign("usrabc123", 1_700_000_000, KEY);
        // 2h later vs 1h budget.
        let err = verify(&token, KEY, 3600, 1_700_000_000 + 7200).expect_err("err");
        assert!(matches!(err, AppError::Unauthenticated));
    }

    #[test]
    fn case_wrong_key_is_rejected() {
        let token = sign("usrabc123", 1_700_000_000, KEY);
        let other: &[u8] = b"deadbeefdeadbeefdeadbeefdeadbeef";
        let err = verify(&token, other, 3600, 1_700_000_010).expect_err("err");
        assert!(matches!(err, AppError::Unauthenticated));
    }

    #[test]
    fn case_malformed_token_is_rejected() {
        assert!(verify("garbage", KEY, 3600, 0).is_err());
        assert!(verify("no-dot", KEY, 3600, 0).is_err());
        assert!(verify("not.base64!!!", KEY, 3600, 0).is_err());
    }
}

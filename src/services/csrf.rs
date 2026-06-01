//! Double-submit-cookie CSRF token.
//!
//! `mint` produces a token bound to (user_id, fresh random nonce) and
//! signed with the same HMAC key the session cookie uses. The token is
//! placed in both a `__Host-csrf` cookie and inside the form (hidden
//! input). `verify` confirms the two values match exactly (constant-time)
//! and that the signature is valid for the current key.
//!
//! Mismatched cookie vs. form means a cross-site attacker is trying to
//! replay a form against the user's session — reject hard.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::error::AppError;

type HmacSha256 = Hmac<Sha256>;

/// Length of the random nonce embedded in the token. 16 bytes = 128 bits,
/// orders of magnitude beyond any forgery budget.
const NONCE_LEN: usize = 16;

/// Mint a fresh CSRF token bound to `user_id`. Random nonce + HMAC over
/// (user_id, nonce); the outer encoding is `{user_id}.{nonce_b64}.{mac_b64}`.
pub fn mint(user_id: &str, key: &[u8]) -> String {
    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);
    let payload = format!("{user_id}.{}", URL_SAFE_NO_PAD.encode(nonce));
    let mac = mac_bytes(key, payload.as_bytes());
    format!("{payload}.{}", URL_SAFE_NO_PAD.encode(mac))
}

/// Verify a form-submitted CSRF token against the cookie-stored copy. Both
/// must (a) be byte-identical and (b) carry a valid signature under `key`.
pub fn verify(form_token: &str, cookie_token: &str, key: &[u8]) -> Result<(), AppError> {
    if form_token
        .as_bytes()
        .ct_eq(cookie_token.as_bytes())
        .unwrap_u8()
        != 1
    {
        return Err(AppError::Forbidden);
    }
    let parts: Vec<&str> = form_token.split('.').collect();
    if parts.len() != 3 {
        return Err(AppError::Forbidden);
    }
    let payload = format!("{}.{}", parts[0], parts[1]);
    let recv_mac = URL_SAFE_NO_PAD
        .decode(parts[2])
        .map_err(|_| AppError::Forbidden)?;
    let exp_mac = mac_bytes(key, payload.as_bytes());
    if recv_mac.ct_eq(&exp_mac).unwrap_u8() != 1 {
        return Err(AppError::Forbidden);
    }
    Ok(())
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
    fn case_round_trip_accepts_matching_form_and_cookie() {
        let t = mint("usrabc123", KEY);
        verify(&t, &t, KEY).expect("ok");
    }

    #[test]
    fn case_mismatched_form_and_cookie_rejected() {
        let a = mint("usrabc123", KEY);
        let b = mint("usrabc123", KEY);
        // Two mints differ (random nonce) — double-submit must fail.
        let err = verify(&a, &b, KEY).expect_err("err");
        assert!(matches!(err, AppError::Forbidden));
    }

    #[test]
    fn case_tampered_payload_rejected() {
        let mut t = mint("usrabc123", KEY);
        // Touch first byte. New char must still be valid for split('.') to keep parts.
        let byte = t.as_bytes()[0];
        let swap = if byte == b'u' { b'v' } else { b'u' };
        unsafe {
            t.as_bytes_mut()[0] = swap;
        }
        // Form and cookie tampered together — bypasses the equality check
        // but must still fail the MAC check.
        let err = verify(&t, &t, KEY).expect_err("err");
        assert!(matches!(err, AppError::Forbidden));
    }

    #[test]
    fn case_wrong_key_rejected() {
        let t = mint("usrabc123", KEY);
        let other: &[u8] = b"deadbeefdeadbeefdeadbeefdeadbeef";
        assert!(verify(&t, &t, other).is_err());
    }

    #[test]
    fn case_garbage_token_rejected() {
        assert!(verify("", "", KEY).is_err());
        assert!(verify("abc", "abc", KEY).is_err());
        assert!(verify("a.b.c.d", "a.b.c.d", KEY).is_err());
    }
}

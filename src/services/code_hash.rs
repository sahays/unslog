//! Argon2id wrapper for invite-code hashing.
//!
//! `Argon2::default()` in v0.5 is argon2id with the OWASP-recommended
//! parameters (m=19 MiB, t=2, p=1) — sufficient for an interactive login
//! handshake on a single-binary local app. Salt is per-hash and random
//! (`SaltString::generate(&mut OsRng)`). Verification goes through
//! `Argon2::verify_password`, which is constant-time over the hash bytes.

use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, SaltString};
use argon2::{Argon2, PasswordVerifier};

use crate::error::AppError;

/// Hash `plain` with argon2id + a fresh random salt. Returns the standard
/// PHC string (`$argon2id$v=19$…`) — opaque to callers; stored verbatim in
/// `users.code_hash`.
pub fn hash(plain: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    let phc = Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .map_err(|e| AppError::Other(anyhow::anyhow!("argon2 hash: {e}")))?
        .to_string();
    Ok(phc)
}

/// Constant-time verify of `plain` against a stored PHC string.
/// Returns `false` for any failure (malformed hash, wrong code, missing
/// salt). Never panics, never logs the candidate.
pub fn verify(plain: &str, phc_string: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(phc_string) else {
        return false;
    };
    Argon2::default()
        .verify_password(plain.as_bytes(), &parsed)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_round_trip_accepts_correct_code() {
        let phc = hash("GPbb5GAnsEQZ").expect("hash ok");
        assert!(phc.starts_with("$argon2id$"));
        assert!(verify("GPbb5GAnsEQZ", &phc));
    }

    #[test]
    fn case_verify_rejects_wrong_code() {
        let phc = hash("GPbb5GAnsEQZ").expect("hash ok");
        assert!(!verify("GPbb5GAnsEQX", &phc));
        assert!(!verify("", &phc));
    }

    #[test]
    fn case_two_hashes_of_same_plain_differ() {
        // Per-hash random salt — distinct PHC strings, same accept set.
        let a = hash("GPbb5GAnsEQZ").expect("a");
        let b = hash("GPbb5GAnsEQZ").expect("b");
        assert_ne!(a, b);
        assert!(verify("GPbb5GAnsEQZ", &a));
        assert!(verify("GPbb5GAnsEQZ", &b));
    }

    #[test]
    fn case_verify_rejects_malformed_hash() {
        assert!(!verify("anything", ""));
        assert!(!verify("anything", "not-a-phc-string"));
        assert!(!verify("anything", "$argon2id$garbage"));
    }
}

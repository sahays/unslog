//! Invite-code primitives — generation, format validation, forensic hint.
//!
//! Codes are 12 chars from `[A-Za-z0-9]` (62 chars) → log2(62^12) ≈ 71.4
//! bits of entropy, which is comfortably above the brute-force budget
//! given the IP-scoped throttle in `services::login_throttle`.
//!
//! Format validation runs *before* any argon2 work — argon2id verification
//! costs ~50 ms per call (intentional), so a malformed body must fail
//! cheaply or the login endpoint becomes a DoS amplifier.

use rand::rngs::OsRng;
use rand::RngCore;

/// Charset used for invite codes. 62 chars → 71.4 bits at length 12.
const ALPHABET: &[u8; 62] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// Wire / storage length of every invite code. Pinned so the regex in the
/// `users.id` CHECK can stay tight and so `hint()` math is fixed.
pub const CODE_LEN: usize = 12;

/// Number of leading chars used in `code_prefix` for login-attempt forensics.
/// Matches the `login_attempts.code_prefix` CHECK (`<= 4`).
pub const PREFIX_LEN: usize = 4;

/// Mint a fresh 12-char invite code from the OS CSPRNG. Modulo bias for
/// `256 % 62 = 8` is acceptable here: ~3 % non-uniformity per char drops
/// total entropy by < 0.1 bit — orders of magnitude inside the budget.
pub fn generate() -> String {
    let mut raw = [0u8; CODE_LEN];
    OsRng.fill_bytes(&mut raw);
    let mut out = [0u8; CODE_LEN];
    for (i, b) in raw.iter().enumerate() {
        out[i] = ALPHABET[(*b as usize) % ALPHABET.len()];
    }
    // SAFETY: ALPHABET is all ASCII.
    std::str::from_utf8(&out)
        .expect("alphabet is ascii")
        .to_string()
}

/// True iff `code` is exactly 12 ASCII alphanumerics. Cheap; call before
/// hashing or DB lookup so malformed input never reaches argon2.
pub fn is_valid_format(code: &str) -> bool {
    code.len() == CODE_LEN && code.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Forensic hint — first 4 + "…" + last 2 chars. Stored on the user row
/// (`code_hint`) and logged as a structured field on login attempts.
/// Reduces brute-force search space by ~36 bits — still ~35 bits of
/// entropy left, well above the throttle's reach.
pub fn hint(code: &str) -> String {
    if code.len() < CODE_LEN {
        return String::new();
    }
    format!("{}…{}", &code[..PREFIX_LEN], &code[CODE_LEN - 2..])
}

/// First-4 chars of the code, for `login_attempts.code_prefix`. Returns
/// empty string for codes shorter than `PREFIX_LEN`.
pub fn prefix(code: &str) -> &str {
    if code.len() < PREFIX_LEN {
        ""
    } else {
        &code[..PREFIX_LEN]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_generate_returns_12_alphanum() {
        let c = generate();
        assert_eq!(c.len(), CODE_LEN);
        assert!(c.bytes().all(|b| b.is_ascii_alphanumeric()));
    }

    #[test]
    fn case_generate_is_non_deterministic() {
        // Two consecutive calls equal is a 1-in-62^12 event; if the OS
        // CSPRNG is wired correctly this assertion never fires.
        assert_ne!(generate(), generate());
    }

    #[test]
    fn case_is_valid_format_accepts_canonical() {
        assert!(is_valid_format("GPbb5GAnsEQZ"));
        assert!(is_valid_format("aaaaaaaaaaaa"));
        assert!(is_valid_format("ZZZZZZZZZZZZ"));
    }

    #[test]
    fn case_is_valid_format_rejects_wrong_length() {
        assert!(!is_valid_format(""));
        assert!(!is_valid_format("abc"));
        assert!(!is_valid_format("aaaaaaaaaaaaa")); // 13
        assert!(!is_valid_format("aaaaaaaaaaa")); // 11
    }

    #[test]
    fn case_is_valid_format_rejects_non_alphanum() {
        assert!(!is_valid_format("aaaaaaaaaaa!"));
        assert!(!is_valid_format("aaaa aaaaaaa"));
        assert!(!is_valid_format("aaaa-aaaaaaa"));
        assert!(!is_valid_format("aaaa_aaaaaaa"));
    }

    #[test]
    fn case_hint_first4_dots_last2() {
        assert_eq!(hint("GPbb5GAnsEQZ"), "GPbb…QZ");
    }

    #[test]
    fn case_hint_returns_empty_for_short_input() {
        assert_eq!(hint(""), "");
        assert_eq!(hint("abc"), "");
    }

    #[test]
    fn case_prefix_returns_first_4() {
        assert_eq!(prefix("GPbb5GAnsEQZ"), "GPbb");
        assert_eq!(prefix(""), "");
        assert_eq!(prefix("abc"), "");
    }
}

//! Tests defending against multi-byte char-boundary panics in
//! `excerpt()`. Other constructor/format behaviors are cosmetic; only
//! the boundary panic is an availability bug worth pinning.

use super::*;

/// Handler availability: a body of exactly EXCERPT_CHARS multi-byte
/// characters must not panic on byte-vs-char miscounting in truncate.
#[test]
fn excerpt_does_not_panic_on_multi_byte_char_boundary() {
    let body: String = "é".repeat(EXCERPT_CHARS);
    let e = JournalEntry::new("usrmaster".into(), "t".into(), body.clone());
    assert_eq!(e.excerpt(), body);
}

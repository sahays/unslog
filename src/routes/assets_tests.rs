use super::*;

fn pdf_with_payload(payload: &[u8]) -> Vec<u8> {
    let mut v = PDF_MAGIC.to_vec();
    v.extend_from_slice(payload);
    v
}

/// Trust-boundary: a well-formed minimal PDF body passes.
#[test]
fn validate_pdf_upload_accepts_well_formed_pdf_with_magic_bytes() {
    validate_pdf_upload(&pdf_with_payload(b"-1.4\n%minimal body")).expect("ok");
}

/// Trust-boundary: a zero-byte upload is rejected before any FS write.
#[test]
fn validate_pdf_upload_rejects_empty_body_so_dead_row_never_reaches_disk() {
    let err = validate_pdf_upload(&[]).expect_err("expected bad request");
    assert!(matches!(err, AppError::BadRequest(_)));
}

/// Disk-safety boundary: an upload above the per-file cap is
/// rejected before save, so a single huge PDF cannot fill the data dir.
#[test]
fn validate_pdf_upload_rejects_oversize_body_so_data_dir_cannot_be_filled() {
    let mut bytes = PDF_MAGIC.to_vec();
    bytes.resize(MAX_ASSET_BYTES + 1, 0);
    let err = validate_pdf_upload(&bytes).expect_err("expected bad request");
    assert!(matches!(err, AppError::BadRequest(_)));
}

/// File-type spoofing defense: a body whose suffix says `.pdf` but
/// whose magic bytes are something else (HTML, EXE, zip, etc.) must
/// be rejected — otherwise extraction downstream fails with a
/// confusing error and a non-PDF byte stream lives at a `.pdf` path.
#[test]
fn validate_pdf_upload_rejects_non_pdf_magic_bytes_so_file_type_cannot_be_spoofed() {
    let err = validate_pdf_upload(b"<!DOCTYPE html><html>").expect_err("html");
    assert!(matches!(err, AppError::BadRequest(_)));
    let err = validate_pdf_upload(b"MZ\x90\x00\x03").expect_err("exe");
    assert!(matches!(err, AppError::BadRequest(_)));
    let err = validate_pdf_upload(b"PK\x03\x04docfile").expect_err("zip");
    assert!(matches!(err, AppError::BadRequest(_)));
}

/// Edge: a 4-byte body of exactly `%PDF` and nothing else passes the
/// magic check (downstream extraction will then fail loudly, but that
/// is the right layer for that error — not this validator).
#[test]
fn validate_pdf_upload_accepts_bare_magic_bytes_defers_structure_check_to_extractor() {
    validate_pdf_upload(PDF_MAGIC).expect("ok");
}

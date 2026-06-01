//! Server-side 32-byte HMAC key, stored at `<data_dir>/session.key`.
//!
//! Used (in Phase 1.2) to sign session cookies and CSRF tokens. Minted on
//! first boot via `OsRng` and persisted with mode `0600` so the key is
//! readable only by the user running the binary. Subsequent boots load
//! the same bytes — rotating the key invalidates every outstanding
//! session, so the operator does it deliberately by deleting the file.

use std::path::{Path, PathBuf};

use rand::rngs::OsRng;
use rand::RngCore;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::error::AppError;

/// Length in bytes of the HMAC-SHA256 server key. 32 bytes = 256 bits =
/// the SHA-256 block size — no truncation in `hmac::Hmac::<Sha256>::new`.
pub const KEY_LEN: usize = 32;

/// Filename under `data_dir` where the key persists.
pub const FILE_NAME: &str = "session.key";

/// Resolve `<data_dir>/session.key`. Centralized so the on-disk path is
/// one constant instead of being reassembled per call site.
pub fn path(data_dir: &str) -> PathBuf {
    Path::new(data_dir).join(FILE_NAME)
}

/// Load the key from disk; mint + persist a new one if absent. Always
/// returns `KEY_LEN` bytes. On Unix the freshly written file is `chmod 600`.
pub async fn load_or_create(data_dir: &str) -> Result<[u8; KEY_LEN], AppError> {
    let p = path(data_dir);
    if let Some(existing) = read_if_present(&p).await? {
        return Ok(existing);
    }
    mint_and_persist(&p).await
}

async fn read_if_present(p: &Path) -> Result<Option<[u8; KEY_LEN]>, AppError> {
    match fs::read(p).await {
        Ok(bytes) if bytes.len() == KEY_LEN => {
            let mut out = [0u8; KEY_LEN];
            out.copy_from_slice(&bytes);
            Ok(Some(out))
        }
        Ok(other) => Err(AppError::Other(anyhow::anyhow!(
            "{} has {} bytes, expected {}",
            p.display(),
            other.len(),
            KEY_LEN
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

async fn mint_and_persist(p: &Path) -> Result<[u8; KEY_LEN], AppError> {
    let mut key = [0u8; KEY_LEN];
    OsRng.fill_bytes(&mut key);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).await?;
    }
    write_locked_down(p, &key).await?;
    tracing::info!(
        event = "auth.session_key.minted",
        path = %p.display(),
        "minted new session key"
    );
    Ok(key)
}

/// Write `bytes` to `p` and (on Unix) `chmod 600` the file. Order: create
/// → set permissions → write. Permissions are set on the file handle so
/// the bytes are written through a 0600 fd, not a 0644-then-chmod fd.
async fn write_locked_down(p: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let mut opts = fs::OpenOptions::new();
    opts.create(true).truncate(true).write(true);
    set_unix_mode_600(&mut opts);
    let mut f = opts.open(p).await?;
    f.write_all(bytes).await?;
    f.flush().await?;
    Ok(())
}

/// On Unix, set the file's creation mode to `0600`. Tokio's `OpenOptions`
/// exposes `mode` directly on the Unix target — no extension trait needed.
#[cfg(unix)]
fn set_unix_mode_600(opts: &mut fs::OpenOptions) {
    opts.mode(0o600);
}

#[cfg(not(unix))]
fn set_unix_mode_600(_opts: &mut fs::OpenOptions) {}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn case_first_load_mints_and_persists() {
        let dir = tempdir().expect("tmp");
        let key = load_or_create(dir.path().to_str().unwrap())
            .await
            .expect("ok");
        assert_eq!(key.len(), KEY_LEN);
        assert!(path(dir.path().to_str().unwrap()).exists());
    }

    #[tokio::test]
    async fn case_second_load_returns_same_bytes() {
        let dir = tempdir().expect("tmp");
        let d = dir.path().to_str().unwrap();
        let a = load_or_create(d).await.expect("a");
        let b = load_or_create(d).await.expect("b");
        assert_eq!(a, b);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn case_persisted_file_is_chmod_600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().expect("tmp");
        let d = dir.path().to_str().unwrap();
        let _ = load_or_create(d).await.expect("ok");
        let meta = std::fs::metadata(path(d)).expect("meta");
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }

    #[tokio::test]
    async fn case_short_file_returns_error() {
        let dir = tempdir().expect("tmp");
        let d = dir.path().to_str().unwrap();
        std::fs::write(path(d), b"short").expect("write");
        let err = load_or_create(d).await.expect_err("err");
        assert!(matches!(err, AppError::Other(_)));
    }
}

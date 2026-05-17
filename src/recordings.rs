//! Filesystem layout helpers for audio recordings.
//!
//! data/
//!   recordings/<company_id>/<session_id>/question_<qid>.mp3
//!                                        critique_<qid>_v<n>.mp3
//!                                        <uuid>.<ext>           (raw answer audio)
//!
//! Mongo stores the *path*; bytes never live in the DB.
//! Filenames are constructed by `routes::sessions` and `services::stt` —
//! this module only exposes the directory + URL helpers they share.

use std::path::{Path, PathBuf};

pub fn session_dir(data_dir: &str, company_id: &str, session_id: &str) -> PathBuf {
    Path::new(data_dir)
        .join("recordings")
        .join(company_id)
        .join(session_id)
}

pub async fn ensure_dir(path: &Path) -> std::io::Result<()> {
    tokio::fs::create_dir_all(path).await
}

/// Convert a stored absolute-or-relative recording path into a server URL like
/// `/recordings/<company>/<session>/<file>`. Returns empty string if the path
/// doesn't sit under any `recordings` dir.
pub fn to_url(path: &str) -> String {
    let needle = "recordings/";
    if let Some(idx) = path.find(needle) {
        let rest = &path[idx + needle.len()..];
        format!("/recordings/{rest}")
    } else {
        String::new()
    }
}

#[cfg(test)]
#[path = "recordings_tests.rs"]
mod tests;

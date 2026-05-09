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
mod tests {
    use super::*;

    #[test]
    fn case_absolute_path_with_recordings() {
        assert_eq!(
            to_url("/data/recordings/c1/s1/x.mp3"),
            "/recordings/c1/s1/x.mp3"
        );
    }

    #[test]
    fn case_no_recordings_substring() {
        assert_eq!(to_url("/data/other/x.mp3"), "");
    }

    #[test]
    fn case_relative_path() {
        assert_eq!(
            to_url("data/recordings/c1/s1/x.mp3"),
            "/recordings/c1/s1/x.mp3"
        );
    }

    #[test]
    fn case_first_match_wins() {
        // Path containing two `recordings/` substrings: `find` returns the
        // first hit, so the remainder includes the second occurrence.
        assert_eq!(
            to_url("/data/recordings/recordings/c/s/x.mp3"),
            "/recordings/recordings/c/s/x.mp3"
        );
    }
}

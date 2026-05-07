//! Filesystem layout helpers for audio recordings.
//!
//! data/
//!   recordings/<company_id>/<session_id>/q<NN>_question.mp3
//!                                        q<NN>_answer_v<N>.webm
//!                                        q<NN>_critique.mp3
//!
//! Mongo stores the *path*; bytes never live in the DB.

use std::path::{Path, PathBuf};

pub fn recordings_root(data_dir: &str) -> PathBuf {
    Path::new(data_dir).join("recordings")
}

pub fn session_dir(data_dir: &str, company_id: &str, session_id: &str) -> PathBuf {
    recordings_root(data_dir).join(company_id).join(session_id)
}

pub fn question_audio_path(
    data_dir: &str,
    company_id: &str,
    session_id: &str,
    q_index: usize,
) -> PathBuf {
    session_dir(data_dir, company_id, session_id).join(format!("q{:02}_question.mp3", q_index))
}

pub fn answer_audio_path(
    data_dir: &str,
    company_id: &str,
    session_id: &str,
    q_index: usize,
    attempt: usize,
    ext: &str,
) -> PathBuf {
    session_dir(data_dir, company_id, session_id)
        .join(format!("q{:02}_answer_v{}.{}", q_index, attempt, ext))
}

pub fn critique_audio_path(
    data_dir: &str,
    company_id: &str,
    session_id: &str,
    q_index: usize,
    attempt: usize,
) -> PathBuf {
    session_dir(data_dir, company_id, session_id)
        .join(format!("q{:02}_critique_v{}.mp3", q_index, attempt))
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

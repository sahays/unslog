use super::*;

/// Handler-context boundary: an allow-listed extension passes through
/// lowercased so the on-disk filename matches what the OS file-type
/// detection expects.
#[test]
fn sanitize_audio_ext_accepts_allowed_ext_and_lowercases_it() {
    assert_eq!(sanitize_audio_ext("webm"), "webm");
    assert_eq!(sanitize_audio_ext("WEBM"), "webm");
    assert_eq!(sanitize_audio_ext("OGG"), "ogg");
    assert_eq!(sanitize_audio_ext("Mp3"), "mp3");
}

/// Filename-suffix attack defense: an extension outside the allow-list
/// must fall back to the default. Otherwise a client-supplied
/// `original.exe` (or `original.sh`, `original.html`) lands on disk
/// with that suffix and downstream tooling may execute it.
#[test]
fn sanitize_audio_ext_rejects_disallowed_ext_so_attacker_chosen_suffix_never_reaches_disk() {
    assert_eq!(sanitize_audio_ext("exe"), DEFAULT_AUDIO_EXT);
    assert_eq!(sanitize_audio_ext("sh"), DEFAULT_AUDIO_EXT);
    assert_eq!(sanitize_audio_ext("html"), DEFAULT_AUDIO_EXT);
    assert_eq!(sanitize_audio_ext("php"), DEFAULT_AUDIO_EXT);
}

/// Filename-suffix attack defense: empty or path-traversal-shaped
/// extension strings must fall back to the default — never propagate.
#[test]
fn sanitize_audio_ext_falls_back_on_empty_or_path_traversal_shaped_input() {
    assert_eq!(sanitize_audio_ext(""), DEFAULT_AUDIO_EXT);
    assert_eq!(sanitize_audio_ext(".."), DEFAULT_AUDIO_EXT);
    assert_eq!(sanitize_audio_ext("/etc/passwd"), DEFAULT_AUDIO_EXT);
}

/// Allow-list contract: the default we fall back to must itself be on
/// the allow-list — otherwise a config drift could fall back to a
/// disallowed extension and the previous tests would pass while real
/// uploads got rejected downstream.
#[test]
fn default_audio_ext_is_a_member_of_the_allow_list() {
    assert!(
        ALLOWED_AUDIO_EXTS.contains(&DEFAULT_AUDIO_EXT),
        "DEFAULT_AUDIO_EXT ({DEFAULT_AUDIO_EXT}) must appear in ALLOWED_AUDIO_EXTS",
    );
}

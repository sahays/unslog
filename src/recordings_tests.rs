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

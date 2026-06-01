use super::url_without_password;

#[test]
fn case_redacts_password_in_standard_url() {
    assert_eq!(
        url_without_password("postgres://u:p@h/db"),
        "postgres://u:***@h/db"
    );
}

#[test]
fn case_redacts_password_with_port_and_query() {
    assert_eq!(
        url_without_password("postgres://unslog:secret@localhost:5432/unslog?sslmode=disable"),
        "postgres://unslog:***@localhost:5432/unslog?sslmode=disable"
    );
}

#[test]
fn case_no_password_returns_userinfo_verbatim() {
    assert_eq!(
        url_without_password("postgres://u@h/db"),
        "postgres://u@h/db"
    );
}

#[test]
fn case_no_userinfo_returns_url_unchanged() {
    assert_eq!(
        url_without_password("postgres://localhost:5432/db"),
        "postgres://localhost:5432/db"
    );
}

#[test]
fn case_malformed_url_returns_verbatim() {
    assert_eq!(url_without_password("not-a-url"), "not-a-url");
    assert_eq!(url_without_password(""), "");
}

#[test]
fn case_authority_only_url_still_redacts() {
    // No path after authority — exercises the no-`/` branch.
    assert_eq!(
        url_without_password("postgres://u:p@host"),
        "postgres://u:***@host"
    );
}

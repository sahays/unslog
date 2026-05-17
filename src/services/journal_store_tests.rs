use super::*;
use mockall::predicate;

fn fixture_entry(id: &str) -> JournalEntry {
    let now = chrono::Utc::now();
    JournalEntry {
        id: id.into(),
        title: "old title".into(),
        body: "old body".into(),
        created_at: now,
        updated_at: now,
        archived_at: None,
    }
}

#[tokio::test]
async fn case_create_inserts_entry_with_equal_timestamps_and_active() {
    let mut deps = MockJournalSource::new();
    deps.expect_insert().times(1).returning(|entry| {
        assert!(!entry.id.is_empty(), "id should be generated");
        assert_eq!(entry.title, "t");
        assert_eq!(entry.body, "b");
        assert_eq!(
            entry.created_at, entry.updated_at,
            "fresh entry has equal timestamps"
        );
        assert!(entry.archived_at.is_none(), "fresh entry is not archived");
        Ok(())
    });

    let out = create(&deps, "t".into(), "b".into()).await.expect("ok");
    assert!(!out.id.is_empty());
}

#[tokio::test]
async fn case_update_bumps_updated_at_and_writes_new_fields() {
    let existing = fixture_entry("e-1");
    let original_created = existing.created_at;
    let original_updated = existing.updated_at;
    let stored = existing.clone();

    let mut deps = MockJournalSource::new();
    deps.expect_find_one()
        .with(predicate::eq("e-1"))
        .times(1)
        .returning(move |_| Ok(Some(stored.clone())));
    deps.expect_replace().times(1).returning(move |entry| {
        assert_eq!(entry.id, "e-1");
        assert_eq!(entry.title, "new title");
        assert_eq!(entry.body, "new body");
        assert_eq!(entry.created_at, original_created, "created_at preserved");
        assert!(
            entry.updated_at > original_updated,
            "updated_at should be bumped"
        );
        Ok(())
    });

    let out = update(&deps, "e-1", "new title".into(), "new body".into())
        .await
        .expect("ok");
    assert_eq!(out.title, "new title");
    assert_eq!(out.body, "new body");
}

#[tokio::test]
async fn case_update_returns_not_found_when_missing() {
    let mut deps = MockJournalSource::new();
    deps.expect_find_one()
        .with(predicate::eq("missing"))
        .times(1)
        .returning(|_| Ok(None));

    let err = update(&deps, "missing", "t".into(), "b".into())
        .await
        .expect_err("expected NotFound");
    assert!(matches!(err, AppError::NotFound(_)));
}

#[tokio::test]
async fn case_archive_sets_archived_at_and_preserves_other_fields() {
    let existing = fixture_entry("e-1");
    let original_title = existing.title.clone();
    let original_body = existing.body.clone();
    let stored = existing.clone();

    let mut deps = MockJournalSource::new();
    deps.expect_find_one()
        .with(predicate::eq("e-1"))
        .times(1)
        .returning(move |_| Ok(Some(stored.clone())));
    deps.expect_replace().times(1).returning(move |entry| {
        assert_eq!(entry.id, "e-1");
        assert!(
            entry.archived_at.is_some(),
            "archived_at should be populated"
        );
        assert_eq!(entry.title, original_title, "title untouched");
        assert_eq!(entry.body, original_body, "body untouched");
        Ok(())
    });

    archive(&deps, "e-1").await.expect("ok");
}

#[tokio::test]
async fn case_archive_returns_not_found_when_missing() {
    let mut deps = MockJournalSource::new();
    deps.expect_find_one()
        .with(predicate::eq("ghost"))
        .times(1)
        .returning(|_| Ok(None));

    let err = archive(&deps, "ghost")
        .await
        .expect_err("expected NotFound");
    assert!(matches!(err, AppError::NotFound(_)));
}

#[tokio::test]
async fn case_list_active_returns_entries_from_source() {
    let mut deps = MockJournalSource::new();
    deps.expect_find_active()
        .times(1)
        .returning(|| Ok(vec![fixture_entry("a"), fixture_entry("b")]));

    let out = list_active(&deps).await.expect("ok");
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].id, "a");
    assert_eq!(out[1].id, "b");
}

#[tokio::test]
async fn case_get_returns_entry_when_present() {
    let mut deps = MockJournalSource::new();
    deps.expect_find_one()
        .with(predicate::eq("e-1"))
        .times(1)
        .returning(|_| Ok(Some(fixture_entry("e-1"))));

    let out = get(&deps, "e-1").await.expect("ok");
    assert!(out.is_some());
    assert_eq!(out.unwrap().id, "e-1");
}

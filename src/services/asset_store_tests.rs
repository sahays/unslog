use super::*;
use crate::models::{Asset, AssetKind, ExtractionStatus};
use mockall::predicate;

fn fixture_asset(id: &str, kind: AssetKind, primary: bool) -> Asset {
    Asset {
        id: id.into(),
        name: format!("asset-{id}"),
        kind,
        primary,
        original_filename: format!("{id}.pdf"),
        original_path: format!("/tmp/{id}.pdf"),
        extracted_path: None,
        extraction_status: ExtractionStatus::Ok,
        extraction_error: None,
        uploaded_at: chrono::Utc::now(),
    }
}

#[tokio::test]
async fn case_set_primary_of_resume_clears_only_resume_peers() {
    let target = fixture_asset("r-2", AssetKind::Resume, false);
    let mut src = MockAssetSource::new();
    src.expect_find_by_id()
        .with(predicate::eq("r-2"))
        .times(1)
        .returning(move |_| Ok(Some(target.clone())));
    src.expect_set_primary_flag()
        .with(predicate::eq("r-2"), predicate::eq(true))
        .times(1)
        .returning(|_, _| Ok(1));
    // Critical: the clear is scoped to Resume — the primary Book must not
    // be touched.
    src.expect_clear_primary_in_kind_excluding()
        .withf(|k, keep| *k == AssetKind::Resume && keep == "r-2")
        .times(1)
        .returning(|_, _| Ok(()));

    set_primary_via(&src, "r-2").await.expect("ok");
}

#[tokio::test]
async fn case_set_primary_of_book_clears_only_book_peers() {
    let target = fixture_asset("b-1", AssetKind::Book, false);
    let mut src = MockAssetSource::new();
    src.expect_find_by_id()
        .with(predicate::eq("b-1"))
        .times(1)
        .returning(move |_| Ok(Some(target.clone())));
    src.expect_set_primary_flag()
        .with(predicate::eq("b-1"), predicate::eq(true))
        .times(1)
        .returning(|_, _| Ok(1));
    src.expect_clear_primary_in_kind_excluding()
        .withf(|k, keep| *k == AssetKind::Book && keep == "b-1")
        .times(1)
        .returning(|_, _| Ok(()));

    set_primary_via(&src, "b-1").await.expect("ok");
}

#[tokio::test]
async fn case_set_primary_returns_not_found_when_missing() {
    let mut src = MockAssetSource::new();
    src.expect_find_by_id()
        .with(predicate::eq("ghost"))
        .times(1)
        .returning(|_| Ok(None));

    let err = set_primary_via(&src, "ghost")
        .await
        .expect_err("expected NotFound");
    assert!(matches!(err, AppError::NotFound(_)));
}

#[tokio::test]
async fn case_delete_primary_resume_promotes_next_resume_not_book() {
    let deleted = fixture_asset("r-1", AssetKind::Resume, true);
    let next_resume = fixture_asset("r-0", AssetKind::Resume, false);
    let mut src = MockAssetSource::new();
    // Scope of the promotion query MUST be Resume — picking a Book here
    // would silently promote the wrong kind and break the per-kind
    // primary invariant.
    src.expect_find_newest_of_kind_excluding()
        .withf(|k, ex| *k == AssetKind::Resume && ex == "r-1")
        .times(1)
        .returning(move |_, _| Ok(Some(next_resume.clone())));
    src.expect_set_primary_flag()
        .with(predicate::eq("r-0"), predicate::eq(true))
        .times(1)
        .returning(|_, _| Ok(1));
    src.expect_delete_by_id()
        .with(predicate::eq("r-1"))
        .times(1)
        .returning(|_| Ok(()));

    delete_via(&src, &deleted).await.expect("ok");
}

#[tokio::test]
async fn case_delete_only_resume_leaves_kind_empty_no_promotion() {
    let deleted = fixture_asset("r-1", AssetKind::Resume, true);
    let mut src = MockAssetSource::new();
    src.expect_find_newest_of_kind_excluding()
        .withf(|k, ex| *k == AssetKind::Resume && ex == "r-1")
        .times(1)
        .returning(|_, _| Ok(None));
    // No set_primary_flag call — no peer to promote.
    src.expect_delete_by_id()
        .with(predicate::eq("r-1"))
        .times(1)
        .returning(|_| Ok(()));

    delete_via(&src, &deleted).await.expect("ok");
}

#[tokio::test]
async fn case_delete_non_primary_skips_promotion_entirely() {
    let deleted = fixture_asset("r-2", AssetKind::Resume, false);
    let mut src = MockAssetSource::new();
    // No promotion lookup, just the row delete.
    src.expect_delete_by_id()
        .with(predicate::eq("r-2"))
        .times(1)
        .returning(|_| Ok(()));

    delete_via(&src, &deleted).await.expect("ok");
}

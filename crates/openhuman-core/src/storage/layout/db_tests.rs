use super::*;
use crate::storage::layout::test_fixtures::{fixture_docs, v1_profile};
use crate::storage::layout::{legacy, Target};
use tempfile::TempDir;

#[test]
fn the_copy_has_the_sources_counts_and_leaves_the_source_alone() {
    let fx = v1_profile();
    let source = legacy::v1_db(&fx.target);
    checkpoint(&source).unwrap();
    let before = std::fs::read(&source).unwrap();
    let dest = fx.target.storage_db();
    let copied = copy(&source, &dest).unwrap();
    assert!(!copied.reused);
    assert_eq!(copied.counts, table_counts(&source).unwrap());
    assert_eq!(copied.counts, table_counts(&dest).unwrap());
    assert!(!dest.with_file_name("storage.db.tmp").exists());
    assert_eq!(
        std::fs::read(&source).unwrap(),
        before,
        "the source is read-only to the copy"
    );
    assert_eq!(fixture_docs(&dest).len(), 3);
}

#[test]
fn a_matching_copy_is_kept_and_a_stale_one_replaced() {
    let fx = v1_profile();
    let source = legacy::v1_db(&fx.target);
    let dest = fx.target.storage_db();
    copy(&source, &dest).unwrap();
    assert!(copy(&source, &dest).unwrap().reused);
    // a leftover that does not match the source is rebuilt
    std::fs::remove_file(&dest).unwrap();
    let stale = Connection::open(&dest).unwrap();
    stale.execute_batch("CREATE TABLE junk (x)").unwrap();
    drop(stale);
    let again = copy(&source, &dest).unwrap();
    assert!(!again.reused);
    assert_eq!(table_counts(&dest).unwrap(), table_counts(&source).unwrap());
}

#[test]
fn a_count_mismatch_names_the_table() {
    let a: Counts = [("docs".to_owned(), 3), ("blobs".to_owned(), 1)].into();
    let b: Counts = [("docs".to_owned(), 2), ("blobs".to_owned(), 1)].into();
    let diff = mismatch(&a, &b).unwrap();
    assert!(diff.contains("docs"), "{diff}");
    assert!(!diff.contains("blobs"), "{diff}");
    assert!(mismatch(&a, &a).is_none());
}

#[test]
fn a_missing_source_is_an_error() {
    let dir = TempDir::new().unwrap();
    let target = Target::new(dir.path(), "u1");
    assert!(copy(&dir.path().join("nope.db"), &target.storage_db()).is_err());
}

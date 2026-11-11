use super::*;
use crate::storage::layout::test_fixtures::{fixture_docs, v1_profile};

#[tokio::test(flavor = "multi_thread")]
async fn the_import_brings_legacy_files_into_the_v1_database() {
    let fx = v1_profile();
    let summary = import(&fx.target, &fx.config)
        .await
        .unwrap_or_else(|error| {
            eprintln!("profile legacy import failed: {error:#}");
            panic!("profile legacy import did not complete");
        });
    assert_eq!(summary.memory_state, vec!["jobs".to_owned()]);
    assert!(
        summary.domains >= 3,
        "approval, devices, ... ran: {summary:?}"
    );

    let jobs = crate::storage::layout::test_fixtures::jobs_value(&v1_db(&fx.target));
    assert_eq!(jobs, Some(serde_json::json!({"jobs": [1, 2, 3]})));
    // the sources are only read
    assert!(fx.workspace().join("memory/jobs.json").is_file());
    // nothing else was disturbed
    assert_eq!(fixture_docs(&v1_db(&fx.target)).len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn importing_twice_changes_nothing() {
    let fx = v1_profile();
    import(&fx.target, &fx.config)
        .await
        .unwrap_or_else(|error| panic!("first legacy import failed: {error:#?}"));
    migrate(&fx.target).await.unwrap();
    let first_fixture_docs = fixture_docs(&v1_db(&fx.target));
    let first_jobs = crate::storage::layout::test_fixtures::jobs_value(&v1_db(&fx.target));
    import(&fx.target, &fx.config)
        .await
        .unwrap_or_else(|error| panic!("second legacy import failed: {error:#?}"));
    migrate(&fx.target).await.unwrap();
    assert_eq!(fixture_docs(&v1_db(&fx.target)), first_fixture_docs);
    assert_eq!(
        crate::storage::layout::test_fixtures::jobs_value(&v1_db(&fx.target)),
        first_jobs
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_damaged_state_file_fails_the_import_before_anything_is_deleted() {
    let fx = v1_profile();
    crate::storage::layout::test_fixtures::write(
        &fx.workspace().join("memory/broken.json"),
        "{ not json",
    );
    let error = import(&fx.target, &fx.config)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("nothing was archived or deleted"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn no_workspace_means_nothing_to_import() {
    let dir = tempfile::TempDir::new().unwrap();
    let target = Target::new(dir.path(), "u1");
    assert_eq!(
        import(&target, &Config::default()).await.unwrap(),
        ImportSummary::default()
    );
    migrate(&target).await.unwrap();
    assert!(!target.workspace().exists());
}

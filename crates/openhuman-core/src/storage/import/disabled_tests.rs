use super::*;

#[test]
fn importing_without_the_feature_reports_unavailable_instead_of_success() {
    let backend: Arc<dyn StorageBackend> =
        Arc::new(tinystoragedrivers_testkit::MemoryBackend::new());
    let reports = run_all(&backend, &Config::default());

    assert_eq!(reports.len(), 1);
    let error = reports[0]
        .outcome
        .as_ref()
        .expect_err("a disabled importer must not claim success");
    assert!(error.to_string().contains("legacy-import"));
}

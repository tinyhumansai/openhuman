use openhuman_core::storage::{backend_has_cross_process_cas, open};

#[tokio::test]
async fn sqlite_backend_reports_cross_process_document_cas() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite:{}", dir.path().join("storage.db").display());
    let backend = open(&url).await.unwrap();

    assert!(backend_has_cross_process_cas(backend.as_ref()));
}

#[tokio::test]
async fn memory_backend_does_not_report_cross_process_document_cas() {
    let backend = open("memory").await.unwrap();

    assert!(!backend_has_cross_process_cas(backend.as_ref()));
}

#[cfg(feature = "storage-file")]
#[tokio::test]
async fn file_backend_does_not_report_cross_process_document_cas() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("file:{}", dir.path().join("storage").display());
    let backend = open(&url).await.unwrap();

    assert!(!backend_has_cross_process_cas(backend.as_ref()));
}

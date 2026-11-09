use super::*;
use tinymemory_integrations::sources::SourceStatus;

#[test]
fn corrupt_legacy_source_state_is_skipped_and_left_in_place() {
    let tmp = tempfile::tempdir().unwrap();
    let file = crate::memory::state_store::legacy_path(tmp.path(), "sources_state");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let corrupt = b"{\"source-1\":";
    std::fs::write(&file, corrupt).unwrap();

    assert!(load(tmp.path()).is_empty());
    assert!(
        file.exists(),
        "the read-only legacy load must not move the file"
    );
    assert_eq!(std::fs::read(file).unwrap(), corrupt);
}

#[test]
fn native_source_state_keeps_the_persisted_host_shape() {
    let tmp = tempfile::tempdir().unwrap();
    let file = path(tmp.path());
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let saved = serde_json::json!({
        "src-1": {
            "last_sync_at": "2025-01-02T03:04:05Z",
            "status": "error",
            "error": "reader failed",
            "items": 7
        }
    });
    std::fs::write(&file, serde_json::to_vec(&saved).unwrap()).unwrap();

    let all = load(tmp.path());
    assert_eq!(all["src-1"].status, SourceStatus::Error);
    assert_eq!(all["src-1"].error.as_deref(), Some("reader failed"));
    assert_eq!(all["src-1"].items, 7);
    assert_eq!(serde_json::to_value(all).unwrap(), saved);
}

use super::*;
use tempfile::TempDir;

fn test_config_in(tmp: &TempDir) -> Config {
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    cfg
}

#[test]
fn config_validation_warns_no_channels() {
    let config = Config::default();
    let mut items = vec![];
    check_config_semantics(&config, &mut items);
    let ch_item = items.iter().find(|i| i.message.contains("channel"));
    assert!(ch_item.is_some());
    assert_eq!(ch_item.unwrap().severity, Severity::Warn);
}

#[test]
fn truncate_for_display_short() {
    let s = "hello";
    assert_eq!(truncate_for_display(s, 10), s);
}

#[test]
fn truncate_for_display_long() {
    let s = "abcdefghijklmnopqrstuvwxyz";
    let truncated = truncate_for_display(s, 5);
    assert!(truncated.starts_with("abcde"));
    assert!(truncated.ends_with("..."));
}

#[test]
fn embedding_provider_validation_accepts_standard_values() {
    assert_eq!(embedding_provider_validation_error("none"), None);
    assert_eq!(embedding_provider_validation_error("openai"), None);
    assert_eq!(
        embedding_provider_validation_error("custom:https://example.com"),
        None
    );
}

#[test]
fn embedding_provider_validation_rejects_empty_custom_url() {
    let err = embedding_provider_validation_error("custom:   ").expect("should fail");
    assert!(err.contains("non-empty URL"), "{err}");
}

#[test]
fn embedding_provider_validation_rejects_non_http_scheme() {
    let err = embedding_provider_validation_error("custom:file:///tmp/model").expect("should fail");
    assert!(err.contains("http/https"), "{err}");
}

#[test]
fn embedding_provider_validation_rejects_malformed_url() {
    let err = embedding_provider_validation_error("custom:not a url").expect("should fail");
    assert!(err.contains("invalid custom provider URL"), "{err}");
}

#[test]
fn model_matches_accepts_exact_and_tagged_variants() {
    assert!(model_matches("bge-m3", "bge-m3"));
    assert!(model_matches("bge-m3:latest", "bge-m3"));
    assert!(model_matches("bge-m3", "bge-m3:latest"));
    assert!(model_matches("bge-m3:v1.0", "bge-m3"));
}

#[test]
fn model_matches_rejects_different_base_models() {
    assert!(!model_matches("nomic-embed-text:latest", "bge-m3"));
    assert!(!model_matches("bge-m3:latest", "nomic-embed-text"));
    assert!(!model_matches("bge-m3:latest", "bge-m3:v1.0"));
}

// ── check_memory_tree_db tests (#2206) ───────────────────────────────────────

/// When the workspace exists but the DB file has never been created,
/// `check_memory_tree_db` should push a `Warn` about the missing file AND
/// still surface the driver's probe result. Drivers that do not use SQLite
/// have no `chunks.db` by design, so the file check must not gate the probe.
#[test]
fn check_memory_tree_db_warns_when_db_missing() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg = test_config_in(&tmp);

    let mut items = vec![];
    check_memory_tree_db(&cfg, &Ok(7), &mut items);

    // Warn item for the missing file (driver-neutral: describes the SQLite
    // artifact as absent, not the driver as broken).
    assert!(
        items
            .iter()
            .any(|i| i.severity == Severity::Warn && i.message.contains("absent")),
        "expected a 'legacy SQLite artifact is absent' warn item; got: {items:?}",
    );
    // Driver probe result is surfaced regardless of file absence.
    assert!(
        items
            .iter()
            .any(|i| i.severity == Severity::Ok && i.message.contains('7')),
        "expected a driver-probe Ok item with count 7; got: {items:?}",
    );
}

/// With the DB file present and the driver's chunk count in hand, the probe
/// should push an `Ok` item carrying that count.
///
/// The file is created directly rather than by opening a store: the count is
/// the async caller's now (`ops::memory_chunk_count`), so this check only ever
/// asks the filesystem whether the path exists.
#[test]
fn check_memory_tree_db_ok_when_accessible() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg = test_config_in(&tmp);

    let db_dir = cfg.workspace_dir.join("memory_tree");
    std::fs::create_dir_all(&db_dir).expect("create memory_tree dir");
    std::fs::write(db_dir.join("chunks.db"), b"").expect("create chunks.db");

    let mut items = vec![];
    check_memory_tree_db(&cfg, &Ok(42), &mut items);

    // There may be a Warn about the SHM file on some platforms; there must
    // be at least one Ok item about the DB being accessible.
    let ok_items: Vec<_> = items
        .iter()
        .filter(|i| i.severity == Severity::Ok && i.category == "memory_tree_db")
        .collect();
    assert!(
        !ok_items.is_empty(),
        "expected at least one Ok memory_tree_db item; got: {items:?}"
    );
    assert!(
        ok_items[0].message.contains("accessible"),
        "unexpected ok message: {}",
        ok_items[0].message
    );
    assert!(
        ok_items[0].message.contains("(42 chunks)"),
        "the driver's count must reach the message: {}",
        ok_items[0].message
    );
}

/// A driver that could not answer is an `Error` item, not a silent zero: "0
/// chunks" from a store nobody could count reads exactly like an empty store.
#[test]
fn check_memory_tree_db_errors_when_the_driver_could_not_count() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg = test_config_in(&tmp);

    let db_dir = cfg.workspace_dir.join("memory_tree");
    std::fs::create_dir_all(&db_dir).expect("create memory_tree dir");
    std::fs::write(db_dir.join("chunks.db"), b"").expect("create chunks.db");

    let mut items = vec![];
    check_memory_tree_db(
        &cfg,
        &Err("driver 'null' does not serve Maintenance".to_string()),
        &mut items,
    );

    let error_items: Vec<_> = items
        .iter()
        .filter(|i| i.severity == Severity::Error && i.category == "memory_tree_db")
        .collect();
    assert_eq!(
        error_items.len(),
        1,
        "expected exactly one Error memory_tree_db item; got: {items:?}"
    );
    assert!(
        error_items[0].message.contains("DB probe failed at"),
        "unexpected error message: {}",
        error_items[0].message
    );
    assert!(
        error_items[0]
            .message
            .contains("does not serve Maintenance"),
        "the driver's own reason must survive into the report: {}",
        error_items[0].message
    );
}

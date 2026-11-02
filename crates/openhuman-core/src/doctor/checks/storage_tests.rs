use super::*;
use crate::core::runtime::ComponentSet;
use crate::doctor::contract::{DoctorCtx, FindingSeverity};

fn context(root: &std::path::Path, url: String) -> DoctorCtx {
    let mut config = crate::config::Config::default();
    config.workspace_dir = root.join("workspace");
    config.action_dir = root.join("actions");
    config.config_path = root.join("config.toml");
    config.storage.url = Some(url);
    DoctorCtx::from_config_with_components(config, ComponentSet::all())
}

#[tokio::test]
async fn sqlite_connect_uses_read_only_probe_and_does_not_create_missing_paths() {
    let root = tempfile::tempdir().expect("temporary storage root");
    let missing = root.path().join("not-created/storage.db");
    let findings = StorageConnect
        .detect(&context(
            root.path(),
            format!("sqlite:{}", missing.display()),
        ))
        .await;
    assert_eq!(findings[0].severity, FindingSeverity::Error);
    assert!(!missing.exists());
    assert!(!missing.parent().expect("parent").exists());
}

#[tokio::test]
async fn corrupt_sqlite_is_an_error_without_echoing_its_location() {
    let root = tempfile::tempdir().expect("temporary storage root");
    let file = root.path().join("private-path.sqlite");
    std::fs::write(&file, b"not a database").expect("write corrupt fixture");
    let before = std::fs::read(&file).expect("read before probe");
    let findings = StorageConnect
        .detect(&context(root.path(), format!("sqlite:{}", file.display())))
        .await;
    assert_eq!(findings[0].severity, FindingSeverity::Error);
    assert!(!findings[0].message.contains("private-path"));
    assert_eq!(std::fs::read(file).expect("read after probe"), before);
}

#[tokio::test]
async fn file_storage_probe_reads_existing_directory_and_never_creates_it() {
    let root = tempfile::tempdir().expect("temporary storage root");
    let existing = root.path().join("file-storage");
    std::fs::create_dir(&existing).expect("create existing fixture root");
    let good = StorageConnect
        .detect(&context(
            root.path(),
            format!("file:{}", existing.display()),
        ))
        .await;
    assert_eq!(good[0].severity, FindingSeverity::Pass);

    let missing = root.path().join("private-storage-root");
    let bad = StorageConnect
        .detect(&context(root.path(), format!("file:{}", missing.display())))
        .await;
    assert_eq!(bad[0].severity, FindingSeverity::Error);
    assert!(!missing.exists());
    assert!(!bad[0].message.contains("private-storage-root"));
}

#[tokio::test]
async fn memory_storage_probe_reports_the_backend_as_available() {
    let findings = inspect_storage_config("storage/connect", StorageConfig::Memory).await;
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].severity, FindingSeverity::Pass);
}

/// Needs a live server: set `OPENHUMAN_TEST_MONGODB_URI` (e.g.
/// `mongodb://127.0.0.1:27017`) and build with `storage-mongodb`.
#[cfg(feature = "storage-mongodb")]
#[tokio::test]
async fn mongodb_connect_probe_creates_nothing_in_the_inspected_database() {
    use std::io::Write as _;
    let Ok(uri) = std::env::var("OPENHUMAN_TEST_MONGODB_URI") else {
        let _ = writeln!(
            std::io::stderr(),
            "skipped: OPENHUMAN_TEST_MONGODB_URI unset"
        );
        return;
    };
    // Unique per run, and dropped below only if the probe itself created it.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let database = format!("oh_doctor_probe_{}_{nanos}", std::process::id());
    let findings = inspect_storage_config(
        "storage/connect",
        StorageConfig::MongoDb {
            uri: uri.clone(),
            database: database.clone(),
        },
    )
    .await;
    let db = tinystoragedrivers_connect::MongoConnection::connect(&uri, &database)
        .await
        .expect("test client")
        .database();
    let created = db.list_collection_names().await.expect("list collections");
    if !created.is_empty() {
        let _ = db.drop().await;
    }
    assert_eq!(findings[0].severity, FindingSeverity::Pass);
    assert!(
        created.is_empty(),
        "the read-only probe created {created:?}"
    );
}

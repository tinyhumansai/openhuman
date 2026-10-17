use super::*;
use crate::storage::import::Outcome;
use crate::storage::{CollectionSpec, Precondition, Scope};
use std::sync::Mutex;
use tempfile::TempDir;

fn config_in(dir: &TempDir) -> Config {
    Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    }
}

/// Records what `rebind_with` installed instead of touching the process slot.
#[derive(Default)]
struct Installed(Mutex<Vec<(String, Arc<dyn StorageBackend>)>>);

impl Installed {
    fn install(&self) -> impl FnOnce(&str, Arc<dyn StorageBackend>) -> usize + '_ {
        |url, backend| {
            self.0.lock().unwrap().push((url.to_string(), backend));
            0
        }
    }

    fn urls(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|(url, _)| url.clone())
            .collect()
    }
}

#[test]
fn a_different_workspace_reopens_the_default_backend_there() {
    let first = TempDir::new().unwrap();
    let second = TempDir::new().unwrap();
    let installed = Installed::default();
    let current = super::super::config::default_url(first.path());

    let rebound = rebind_with(&config_in(&second), Some(current), installed.install())
        .unwrap()
        .expect("the workspace moved");

    assert_eq!(
        installed.urls(),
        [super::super::config::default_url(second.path())]
    );
    assert!(rebound.to.ends_with("/storage"), "{}", rebound.to);
    assert!(
        second.path().join("storage").is_dir(),
        "the new workspace got its storage directory"
    );
}

#[test]
fn the_new_workspace_is_a_different_database() {
    let first = TempDir::new().unwrap();
    let second = TempDir::new().unwrap();
    let installed = Installed::default();
    let current = super::super::config::default_url(first.path());
    rebind_with(&config_in(&second), Some(current), installed.install())
        .unwrap()
        .unwrap();
    let backend = Arc::clone(&installed.0.lock().unwrap()[0].1);

    // A record written for the second workspace is not in the first one's.
    let scoped = backend.for_scope(&Scope::local()).unwrap();
    let docs = Arc::clone(scoped.documents());
    crate::storage::block_on(async move {
        docs.ensure_collection(&CollectionSpec::new("probe"))
            .await?;
        docs.put(
            "probe",
            "k",
            serde_json::json!({"v": 1}),
            Precondition::None,
        )
        .await
    })
    .unwrap();

    let other = super::super::block_on_anyhow({
        let url = super::super::config::default_url(first.path());
        async move { Ok(super::super::open(&url).await?) }
    })
    .unwrap();
    let docs = Arc::clone(other.for_scope(&Scope::local()).unwrap().documents());
    let found = crate::storage::block_on(async move {
        docs.ensure_collection(&CollectionSpec::new("probe"))
            .await?;
        docs.get("probe", "k").await
    })
    .unwrap();
    assert!(found.is_none(), "the first workspace never saw the record");
}

#[test]
fn the_legacy_files_of_the_new_workspace_are_imported() {
    let first = TempDir::new().unwrap();
    let second = TempDir::new().unwrap();
    // A user who last ran an older build in `second` has a devices database.
    let db = second.path().join("devices").join("devices.db");
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    let conn = crate::storage::rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE paired_devices (
             channel_id TEXT PRIMARY KEY, label TEXT NOT NULL, device_pubkey TEXT NOT NULL,
             core_session_token_hash TEXT NOT NULL, shared_secret_encrypted BLOB,
             created_at TEXT NOT NULL, last_seen_at TEXT, revoked INTEGER NOT NULL DEFAULT 0
         );
         INSERT INTO paired_devices (channel_id, label, device_pubkey, core_session_token_hash, created_at)
         VALUES ('chan-a', 'Phone', 'pk', 'hash', '2026-01-01T00:00:00+00:00');",
    )
    .unwrap();
    drop(conn);
    let installed = Installed::default();
    let current = super::super::config::default_url(first.path());

    let rebound = rebind_with(&config_in(&second), Some(current), installed.install())
        .unwrap()
        .unwrap();

    let devices = rebound
        .imports
        .iter()
        .find(|report| report.domain == "devices")
        .expect("the devices importer ran");
    assert!(matches!(
        devices.outcome,
        Ok(Outcome::Imported {
            found: 1,
            written: 1
        })
    ));
}

#[test]
fn an_explicit_url_is_never_replaced() {
    let first = TempDir::new().unwrap();
    let second = TempDir::new().unwrap();
    let installed = Installed::default();
    let mut config = config_in(&second);
    config.storage.url = Some("memory".into());

    // The host installed `memory` from the same explicit URL: nothing moved.
    let rebound = rebind_with(&config, Some("memory".into()), installed.install()).unwrap();

    assert!(rebound.is_none());
    assert!(installed.urls().is_empty());
    let _ = first;
}

#[test]
fn the_same_workspace_is_left_alone() {
    let dir = TempDir::new().unwrap();
    let installed = Installed::default();
    let current = super::super::config::default_url(dir.path());

    let rebound = rebind_with(&config_in(&dir), Some(current), installed.install()).unwrap();

    assert!(rebound.is_none());
    assert!(installed.urls().is_empty());
}

#[test]
fn a_backend_installed_without_a_url_is_never_rebound() {
    let dir = TempDir::new().unwrap();
    let installed = Installed::default();

    let rebound = rebind_with(&config_in(&dir), None, installed.install()).unwrap();

    assert!(rebound.is_none());
    assert!(installed.urls().is_empty());
}

#[test]
fn a_backend_that_cannot_be_opened_keeps_the_previous_one() {
    let first = TempDir::new().unwrap();
    let second = TempDir::new().unwrap();
    // The storage directory cannot be created: a file is in the way.
    std::fs::write(second.path().join("storage"), b"in the way").unwrap();
    let installed = Installed::default();
    let current = super::super::config::default_url(first.path());

    let error = rebind_with(&config_in(&second), Some(current), installed.install());

    assert!(error.is_err());
    assert!(installed.urls().is_empty(), "nothing was installed");
}

#[test]
fn rebinding_to_a_fresh_v2_profile_opens_its_storage_db_and_leaves_a_v1_one_alone() {
    let root = TempDir::new().unwrap();
    let v1 = TempDir::new().unwrap();
    let layout = crate::config::schema::ProfileLayout::resolve(root.path(), "alice");
    std::fs::create_dir_all(&layout.workspace_dir).unwrap();
    let installed = Installed::default();
    let current = super::super::config::default_url(v1.path());

    let config = Config {
        workspace_dir: layout.workspace_dir.clone(),
        config_path: layout.config_path.clone(),
        ..Config::default()
    };
    let rebound = rebind_with(&config, Some(current), installed.install())
        .unwrap()
        .expect("the workspace moved");

    assert_eq!(
        installed.urls(),
        [format!("sqlite:{}", layout.storage_db.display())]
    );
    assert!(rebound.to.ends_with("/storage.db"), "{}", rebound.to);
    assert!(layout.storage_db.is_file(), "the profile got its database");
    assert!(
        !layout.workspace_dir.join("storage").exists(),
        "no v1 storage directory under files/"
    );
    assert!(
        !v1.path().join("storage").exists(),
        "the old profile is untouched"
    );
}

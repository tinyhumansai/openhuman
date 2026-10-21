use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use openhuman_core::config::Config;
use openhuman_core::doctor::{
    register_check_source_in, DoctorCheck, DoctorCheckSource, DoctorCtx, DoctorTier, Finding,
};
use openhuman_core::security::secret_storage::{
    clear_master_key_source, clear_secret_backend, install_master_key_source,
    install_secret_backend, EncryptedFileBackend, MasterKeySource, SecretStorageError,
};
use openhuman_embed::{
    ConfigSource, CoreBootMode, DomainSet, RuntimeBuilder, ServiceSet, TokenSource, Workspace,
};
use serde_json::json;
use zeroize::Zeroizing;

#[derive(Debug, Clone, PartialEq, Eq)]
enum SnapshotEntry {
    Directory,
    File(Vec<u8>),
    Symlink(PathBuf),
    Other,
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, SnapshotEntry> {
    fn visit(root: &Path, dir: &Path, entries: &mut BTreeMap<PathBuf, SnapshotEntry>) {
        for entry in fs::read_dir(dir).expect("read snapshot directory") {
            let entry = entry.expect("read snapshot entry");
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            let file_type = entry.file_type().expect("inspect snapshot entry");
            if file_type.is_dir() {
                entries.insert(relative, SnapshotEntry::Directory);
                visit(root, &path, entries);
            } else if file_type.is_file() {
                entries.insert(
                    relative,
                    SnapshotEntry::File(fs::read(&path).expect("read snapshot file")),
                );
            } else if file_type.is_symlink() {
                entries.insert(
                    relative,
                    SnapshotEntry::Symlink(fs::read_link(&path).expect("read symlink")),
                );
            } else {
                entries.insert(relative, SnapshotEntry::Other);
            }
        }
    }

    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries);
    entries
}

struct Environment(Vec<(&'static str, Option<OsString>)>);

impl Environment {
    fn isolated(root: &Path, storage_url: &str) -> Self {
        let overrides = [
            ("OPENHUMAN_WORKSPACE", None),
            (
                "OPENHUMAN_PROJECTS_DIR",
                Some(root.join("projects").into_os_string()),
            ),
            (
                "OPENHUMAN_ACTION_DIR",
                Some(root.join("action").into_os_string()),
            ),
            ("OPENHUMAN_STORAGE_URL", Some(OsString::from(storage_url))),
            (
                "OPENHUMAN_KEYRING_BACKEND",
                Some(OsString::from("encrypted_file")),
            ),
            ("OPENHUMAN_KEYRING_MASTER_KEY", None),
            ("OPENHUMAN_KEYRING_MASTER_KEY_FILE", None),
        ];
        let saved = overrides
            .iter()
            .map(|(key, value)| {
                let previous = std::env::var_os(key);
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
                (*key, previous)
            })
            .collect();
        Self(saved)
    }
}

impl Drop for Environment {
    fn drop(&mut self) {
        for (key, value) in self.0.drain(..) {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

struct SecretBackendReset;

impl Drop for SecretBackendReset {
    fn drop(&mut self) {
        clear_master_key_source();
        clear_secret_backend();
    }
}

struct InspectionCheck;

#[async_trait::async_trait]
impl DoctorCheck for InspectionCheck {
    fn id(&self) -> &str {
        "inspection/probe"
    }

    fn category(&self) -> &str {
        "inspection"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Static
    }

    async fn detect(&self, _ctx: &DoctorCtx) -> Vec<Finding> {
        vec![Finding::pass(self.id(), "Inspection context is available.")]
    }
}

struct InspectionSource;

impl DoctorCheckSource for InspectionSource {
    fn id(&self) -> &str {
        "inspection-source"
    }

    fn checks(&self) -> Vec<Arc<dyn DoctorCheck>> {
        vec![Arc::new(InspectionCheck)]
    }
}

struct CountingMasterKeySource(Arc<AtomicUsize>);

impl MasterKeySource for CountingMasterKeySource {
    fn load(&self) -> Result<Option<Zeroizing<[u8; 32]>>, SecretStorageError> {
        Ok(None)
    }

    fn store(&self, _key: &[u8; 32]) -> Result<(), SecretStorageError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn preloaded_runtime_can_invoke_doctor_without_startup_writes() {
    let runtime = openhuman_embed::process::tokio_runtime().expect("tuned Tokio runtime");
    let task = runtime.spawn(preloaded_runtime_report());
    runtime
        .block_on(task)
        .expect("inspection test task completes");
}

async fn preloaded_runtime_report() {
    let root = tempfile::tempdir().expect("inspection root");
    let workspace = root.path().join("workspace");
    let action = root.path().join("action");
    let storage_file = root.path().join("configured.sqlite");
    let storage_url = format!("sqlite:{}", storage_file.display());
    let _environment = Environment::isolated(root.path(), &storage_url);

    let legacy_skill = workspace.join("skills/dev-workflow/keep.txt");
    fs::create_dir_all(legacy_skill.parent().unwrap()).expect("legacy skill fixture");
    fs::write(&legacy_skill, b"preserve this directory").expect("legacy skill sentinel");

    let mut config = Config {
        config_path: root.path().join("config.toml"),
        workspace_dir: workspace,
        action_dir: action,
        ..Config::default()
    };
    config.storage.url = Some(storage_url);

    let key_writes = Arc::new(AtomicUsize::new(0));
    install_secret_backend(Arc::new(EncryptedFileBackend::new(&config.workspace_dir)));
    install_master_key_source(Arc::new(CountingMasterKeySource(Arc::clone(&key_writes))));
    let _secret_backend = SecretBackendReset;

    let before = snapshot(root.path());
    assert!(openhuman_core::storage::installed().is_none());
    let runtime = RuntimeBuilder::new()
        .workspace(Workspace::Inherit)
        .config_source(ConfigSource::Resolved)
        .config(config)
        .boot_mode(CoreBootMode::Inspection)
        .token(TokenSource::Fixed(Arc::new(
            "inspection-test-token".to_owned(),
        )))
        .domains(DomainSet::full())
        .services(ServiceSet::none())
        .build()
        .await
        .expect("build preloaded inspection runtime");

    assert!(
        openhuman_core::core::runtime::CoreContext::current().is_none(),
        "inspection runtime must not publish a process-default context"
    );
    let backend_opened = openhuman_core::storage::installed().is_some();
    let key_store_writes = key_writes.load(Ordering::SeqCst);
    register_check_source_in(
        runtime.core_runtime().context().extensions(),
        InspectionSource,
    );
    let response = runtime
        .core_runtime()
        .invoke(
            "openhuman.doctor_report",
            json!({
                "tier": "static",
                "only": ["inspection/probe"]
            }),
        )
        .await
        .expect("invoke doctor report in-process");
    assert!(
        openhuman_core::core::runtime::CoreContext::current().is_none(),
        "in-process inspection dispatch must leave no ambient context behind"
    );
    let report = response
        .get("result")
        .expect("unwrap the CLI-compatible outcome envelope");
    assert_eq!(report["schema_version"], 2);
    assert!(report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding["check_id"] == "inspection/probe"));

    drop(runtime);
    let after = snapshot(root.path());
    let changed_paths = before
        .keys()
        .chain(after.keys())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|path| before.get(path) != after.get(path))
        .collect::<Vec<_>>();
    assert!(
        !backend_opened && key_store_writes == 0 && changed_paths.is_empty(),
        "inspection startup side effects: backend_opened={backend_opened}, \
         key_store_writes={key_store_writes}, changed_paths={changed_paths:?}"
    );
}

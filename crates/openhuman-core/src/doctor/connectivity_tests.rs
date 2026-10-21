use std::sync::Arc;

use async_trait::async_trait;

use super::contract::{DoctorCtx, DoctorOptions, FindingSeverity};
use super::runner;
use crate::config::schema::StorageConfig;
use crate::config::Config;

struct DownMemoryEngine {
    inner: tinymemory_api::conformance::ReferenceEngine,
    reason: String,
}

#[async_trait]
impl tinymemory_api::MemoryEngine for DownMemoryEngine {
    fn descriptor(&self) -> &tinymemory_api::EngineDescriptor {
        self.inner.descriptor()
    }

    async fn health(&self) -> tinymemory_api::EngineHealth {
        tinymemory_api::EngineHealth::Down(self.reason.clone())
    }

    async fn recall(
        &self,
        request: tinymemory_api::RecallRequest,
    ) -> tinymemory_api::Result<tinymemory_api::RecallAnswer> {
        self.inner.recall(request).await
    }

    async fn fetch(
        &self,
        request: tinymemory_api::FetchRequest,
    ) -> tinymemory_api::Result<tinymemory_api::FetchPage> {
        self.inner.fetch(request).await
    }

    async fn store(
        &self,
        item: tinymemory_api::StoreItem,
    ) -> tinymemory_api::Result<tinymemory_api::StoreReceipt> {
        self.inner.store(item).await
    }

    async fn forget(
        &self,
        target: tinymemory_api::ForgetTarget,
    ) -> tinymemory_api::Result<tinymemory_api::ForgetReport> {
        self.inner.forget(target).await
    }

    async fn list(
        &self,
        request: tinymemory_api::ListRequest,
    ) -> tinymemory_api::Result<tinymemory_api::ListPage> {
        self.inner.list(request).await
    }
}

fn snapshot_test_tree(
    root: &std::path::Path,
) -> std::collections::BTreeMap<std::path::PathBuf, Option<Vec<u8>>> {
    // SQLite read-only opens may update the derived `-shm` index or create
    // an empty `-wal`; the durable database and committed WAL remain pinned.
    fn walk(
        current: &std::path::Path,
        root: &std::path::Path,
        snapshot: &mut std::collections::BTreeMap<std::path::PathBuf, Option<Vec<u8>>>,
    ) {
        for entry in std::fs::read_dir(current).expect("read fixture directory") {
            let path = entry.expect("fixture entry").path();
            let relative = path
                .strip_prefix(root)
                .expect("fixture path is inside its root")
                .to_path_buf();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            let is_sqlite_bookkeeping = name.ends_with("-shm")
                || (name.ends_with("-wal")
                    && std::fs::metadata(&path).expect("fixture metadata").len() == 0);
            if is_sqlite_bookkeeping {
                continue;
            }
            if path.is_dir() {
                snapshot.insert(relative, None);
                walk(&path, root, snapshot);
            } else {
                snapshot.insert(
                    relative,
                    Some(std::fs::read(path).expect("read fixture file")),
                );
            }
        }
    }

    let mut snapshot = std::collections::BTreeMap::new();
    walk(root, root, &mut snapshot);
    snapshot
}

#[tokio::test]
async fn memory_health_check_reports_the_engine_down_reason() {
    let temp = tempfile::tempdir().expect("temp directory");
    let config = crate::memory::test_fixtures::config_in(&temp);
    let reason = "the configured memory service is unavailable";
    crate::memory::engine::install_test_engine(
        &config.workspace_dir,
        Arc::new(DownMemoryEngine {
            inner: tinymemory_api::conformance::ReferenceEngine::new(),
            reason: reason.to_owned(),
        }),
    );

    let report = runner::run(
        &DoctorCtx::from_config(config.clone()),
        DoctorOptions {
            only: vec!["memory/health".into()],
            ..DoctorOptions::default()
        },
    )
    .await
    .expect("memory health result is represented in the report");
    crate::memory::engine::remove_test_engine(&config.workspace_dir);

    let finding = report
        .findings
        .iter()
        .find(|finding| finding.check_id == "memory/health")
        .expect("the selected memory health check returns its own finding");
    assert_eq!(finding.severity, FindingSeverity::Error);
    assert!(finding.message.contains(reason));
}

#[tokio::test]
async fn storage_connect_check_does_not_change_the_selected_sqlite_tree() {
    let temp = tempfile::tempdir().expect("temp directory");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let database_dir = temp.path().join("selected-storage");
    let url = format!("sqlite:{}", database_dir.display());
    let backend = crate::storage::open(&url)
        .await
        .expect("create a local SQLite fixture using the normal setup path");
    drop(backend);
    let before = snapshot_test_tree(temp.path());
    let config = Config {
        action_dir: workspace.clone(),
        workspace_dir: workspace,
        storage: StorageConfig {
            url: Some(url),
            ..StorageConfig::default()
        },
        ..Config::default()
    };

    let report = runner::run(
        &DoctorCtx::from_config(config),
        DoctorOptions {
            only: vec!["storage/connect".into()],
            ..DoctorOptions::default()
        },
    )
    .await
    .expect("storage check is represented in the report");
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.check_id == "storage/connect")
        .expect("the selected storage check returns its own finding");
    assert_eq!(finding.severity, FindingSeverity::Pass);
    assert_eq!(before, snapshot_test_tree(temp.path()));
}

#[tokio::test]
async fn storage_connect_check_reports_corrupt_sqlite_without_changing_it() {
    let temp = tempfile::tempdir().expect("temp directory");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let file = temp.path().join("corrupt.db");
    std::fs::write(&file, b"not a sqlite database").expect("write corrupt database fixture");
    let before = snapshot_test_tree(temp.path());
    let config = Config {
        action_dir: workspace.clone(),
        workspace_dir: workspace,
        storage: StorageConfig {
            url: Some(format!("sqlite:{}", file.display())),
            ..StorageConfig::default()
        },
        ..Config::default()
    };

    let report = runner::run(
        &DoctorCtx::from_config(config),
        DoctorOptions {
            only: vec!["storage/connect".into()],
            ..DoctorOptions::default()
        },
    )
    .await
    .expect("storage failure is represented in the report");
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.check_id == "storage/connect")
        .expect("the selected storage check returns its own finding");

    assert_eq!(finding.severity, FindingSeverity::Error);
    assert!(!finding.message.contains(file.to_string_lossy().as_ref()));
    assert_eq!(before, snapshot_test_tree(temp.path()));
}

#[tokio::test]
async fn storage_layout_check_warns_when_a_migration_is_pending() {
    let temp = tempfile::tempdir().expect("temp directory");
    let target = crate::storage::layout::Target::new(temp.path(), "u1");
    let workspace = target.workspace();
    std::fs::create_dir_all(&workspace).expect("create workspace");
    std::fs::create_dir_all(&target.profile_dir).expect("create profile directory");
    let config = Config {
        action_dir: workspace.clone(),
        config_path: target.profile_dir.join("config.toml"),
        workspace_dir: workspace,
        ..Config::default()
    };
    let state_path = target
        .profile_dir
        .join(crate::storage::layout::state::STATE_FILE);
    let error = crate::storage::layout::migrate(
        &target,
        &config,
        &crate::storage::layout::Options {
            crash_after: Some(crate::storage::layout::Step::Import),
            ..crate::storage::layout::Options::default()
        },
    )
    .await
    .expect_err("the fixture migration should stop after its import step");
    assert!(matches!(
        error,
        crate::storage::layout::LayoutError::InjectedCrash(crate::storage::layout::Step::Import)
    ));
    let before = std::fs::read(&state_path).expect("read migration state");

    let report = runner::run(
        &DoctorCtx::from_config(config),
        DoctorOptions {
            only: vec!["storage/layout".into()],
            ..DoctorOptions::default()
        },
    )
    .await
    .expect("layout check is represented in the report");

    let finding = report
        .findings
        .iter()
        .find(|finding| finding.check_id == "storage/layout")
        .expect("the selected layout check returns its own finding");
    assert_eq!(finding.severity, FindingSeverity::Warn);
    assert_eq!(
        before,
        std::fs::read(state_path).expect("migration state remains readable")
    );
}

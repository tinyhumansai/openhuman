//! The catalog's storage and the import of the files tinyflows used to write
//! (`flows.db`, `checkpoints.db`, `drafts/*.json`) by the storage importer.

use super::*;
use tempfile::TempDir;

fn fixture_workspace() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/storage/import/fixtures/tinyflows/workspace")
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn config(tmp: &TempDir) -> Config {
    Config {
        workspace_dir: tmp.path().to_path_buf(),
        ..Config::default()
    }
}

#[test]
fn the_catalog_lives_in_the_workspace_default_database() {
    let tmp = TempDir::new().unwrap();
    let config = config(&tmp);
    assert!(crate::storage::installed().is_none());
    assert!(list_flows(&config).unwrap().0.is_empty());
    assert!(tmp.path().join("storage/storage.db").is_file());
}

#[test]
fn legacy_flows_drafts_and_checkpoints_are_imported_and_left_untouched() {
    let tmp = TempDir::new().unwrap();
    let config = config(&tmp);
    copy_tree(&fixture_workspace(), tmp.path());
    let flows_db = dir(&config).join("flows.db");
    let before = std::fs::read(&flows_db).unwrap();

    let backend = crate::storage::local::backend_for(&config.workspace_dir).unwrap();
    for report in crate::storage::import::tinyflows::run_with(&backend, &config) {
        report.outcome.unwrap();
    }

    let (flows, _) = list_flows(&config).unwrap();
    let mut ids: Vec<_> = flows.iter().map(|flow| flow.id.clone()).collect();
    ids.sort();
    assert_eq!(ids, ["flow-alpha", "flow-beta"]);
    assert_eq!(
        super::super::draft_store::list_drafts(&config)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(list_revisions(&config, "flow-alpha", 10).unwrap().len(), 2);
    assert_eq!(
        get_flow_run(&config, "run-done").unwrap().unwrap().flow_id,
        "flow-alpha"
    );

    let checkpointer = crate::flows::tinyflows::open_flow_checkpointer(&config).unwrap();
    let thread = crate::storage::block_on_anyhow(async move {
        Ok(checkpointer.get_thread("run-1").await?.len())
    })
    .unwrap();
    assert!(thread > 0, "the interrupted run's checkpoints came across");

    assert_eq!(
        std::fs::read(&flows_db).unwrap(),
        before,
        "flows.db was modified"
    );

    // Imported once: a flow removed afterwards stays removed after a restart.
    remove_flow(&config, "flow-alpha").unwrap();
    for report in crate::storage::import::tinyflows::run_with(&backend, &config) {
        report.outcome.unwrap();
    }
    assert_eq!(list_flows(&config).unwrap().0.len(), 1);
}

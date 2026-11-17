use super::*;

use crate::config::MemoryLayoutMode;

const HOST_ID: &str = "local-macbook-pro";

/// A local session config at `<tmp>/<install>/users/local-macbook-pro/`.
fn local_config(tmp: &tempfile::TempDir, install: &str) -> Config {
    let user_dir = tmp.path().join(install).join("users").join(HOST_ID);
    let workspace = user_dir.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    Config {
        config_path: user_dir.join("config.toml"),
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        ..Config::default()
    }
}

fn forget_cached(config: &Config) {
    CACHE.lock().unwrap().remove(&path(&config.workspace_dir));
}

fn recorded(config: &Config) -> LocalRoot {
    read(&config.workspace_dir).unwrap().unwrap()
}

#[test]
fn a_fresh_install_mints_a_random_root_and_keeps_it() {
    let tmp = tempfile::tempdir().unwrap();
    let config = local_config(&tmp, "a");
    let root = resolve(&config, HOST_ID).unwrap();
    let id = root.strip_prefix(LOCAL_ROOT_PREFIX).unwrap();
    assert_eq!(id.len(), 32, "a UUID: {root}");
    assert!(!root.contains("macbook"));
    assert_ne!(root, legacy_root(HOST_ID));
    assert_eq!(
        recorded(&config),
        LocalRoot {
            root: root.clone(),
            origin: Origin::Minted
        }
    );

    // A new process reads the same root back from the workspace.
    forget_cached(&config);
    assert_eq!(resolve(&config, HOST_ID).as_deref(), Some(root.as_str()));
}

#[test]
fn two_machines_with_the_same_hostname_get_different_roots() {
    let tmp = tempfile::tempdir().unwrap();
    let first = resolve(&local_config(&tmp, "machine-1"), HOST_ID).unwrap();
    let second = resolve(&local_config(&tmp, "machine-2"), HOST_ID).unwrap();
    assert_ne!(first, second);
}

#[test]
fn an_install_already_on_layout_v3_keeps_its_hostname_root() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = local_config(&tmp, "a");
    config.memory.layout = MemoryLayoutMode::V3;
    let root = resolve(&config, HOST_ID).unwrap();
    assert_eq!(root, legacy_root(HOST_ID), "its memory is still found");
    assert_eq!(recorded(&config).origin, Origin::Legacy);

    // The mapping is recorded once: it holds even when nothing else says so.
    forget_cached(&config);
    config.memory.layout = MemoryLayoutMode::Legacy;
    assert_eq!(resolve(&config, HOST_ID), Some(root));
}

#[test]
fn an_install_mid_migration_keeps_its_hostname_root() {
    let tmp = tempfile::tempdir().unwrap();
    let config = local_config(&tmp, "a");
    crate::memory::layout_migration::state::save(&config.workspace_dir, &Default::default())
        .unwrap();
    assert_eq!(resolve(&config, HOST_ID), Some(legacy_root(HOST_ID)));
}

#[test]
fn the_legacy_root_is_the_old_hashed_user_dir_name() {
    let root = legacy_root("local-megamind-macbook");
    assert!(root.starts_with(LOCAL_ROOT_PREFIX));
    assert_eq!(root.len(), LOCAL_ROOT_PREFIX.len() + 16);
    assert!(!root.contains("megamind"));
    assert_eq!(root, legacy_root("local-megamind-macbook"));
}

#[test]
fn a_record_already_written_wins_a_race() {
    let tmp = tempfile::tempdir().unwrap();
    let config = local_config(&tmp, "a");
    let first = LocalRoot {
        root: format!("{LOCAL_ROOT_PREFIX}aaaa"),
        origin: Origin::Minted,
    };
    let second = LocalRoot {
        root: format!("{LOCAL_ROOT_PREFIX}bbbb"),
        origin: Origin::Minted,
    };
    assert_eq!(record_once(&config.workspace_dir, &first).unwrap(), first);
    assert_eq!(record_once(&config.workspace_dir, &second).unwrap(), first);
    assert_eq!(resolve(&config, HOST_ID), Some(first.root));
}

#[test]
fn a_malformed_record_turns_memory_off_and_is_not_replaced() {
    let tmp = tempfile::tempdir().unwrap();
    let config = local_config(&tmp, "a");
    let file = path(&config.workspace_dir);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, r#"{"root":"user:local-../x","origin":"minted"}"#).unwrap();
    assert_eq!(resolve(&config, HOST_ID), None);
    assert!(std::fs::read_to_string(&file).unwrap().contains("../x"));
}

#[test]
fn a_recorded_root_lives_in_the_state_store_not_a_file() {
    let tmp = tempfile::tempdir().unwrap();
    let config = local_config(&tmp, "a");
    let root = resolve(&config, HOST_ID).unwrap();
    assert!(!path(&config.workspace_dir).exists());
    let stored: LocalRoot = crate::memory::state_store::load(&config.workspace_dir, "local_root")
        .unwrap()
        .unwrap();
    assert_eq!(stored.root, root);
}

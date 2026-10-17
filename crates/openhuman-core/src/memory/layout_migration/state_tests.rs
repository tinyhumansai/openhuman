use tinymemory_tools::layout_migration::{Failure, Phase};

use super::*;

#[test]
fn a_missing_file_is_a_fresh_state() {
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(load(tmp.path()).unwrap(), MigrationState::default());
}

#[test]
fn a_saved_state_loads_back() {
    let tmp = tempfile::tempdir().unwrap();
    let state = MigrationState {
        phase: Phase::Paused,
        cursor: Some("l00ff".into()),
        copied: 7,
        replayed: 2,
        failures: vec![Failure {
            id: "abc".into(),
            reason: "invalid_request".into(),
        }],
        incomplete: vec!["def".into()],
        error: Some("background work is paused".into()),
        cleaning: true,
        switched: true,
        caught_up: false,
        takeover: true,
        rechecks: 1,
        rechecked: true,
    };
    save(tmp.path(), &state).unwrap();
    assert_eq!(load(tmp.path()).unwrap(), state);
    assert!(exists(tmp.path()));
}

#[test]
fn a_damaged_legacy_file_is_an_error_not_a_fresh_start() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("memory")).unwrap();
    let legacy = crate::memory::state_store::legacy_path(tmp.path(), "layout_migration");
    std::fs::write(&legacy, b"{ not json").unwrap();
    assert!(exists(tmp.path()), "a state that cannot be read was begun");
    assert!(load(tmp.path()).is_err());
    // Through the port the host hands the module, too.
    assert!(FileStateStore::new(tmp.path()).load().is_err());
}

#[test]
fn the_port_saves_where_the_status_reads() {
    let tmp = tempfile::tempdir().unwrap();
    let store = FileStateStore::new(tmp.path());
    let state = MigrationState {
        phase: Phase::Copying,
        copied: 3,
        ..MigrationState::default()
    };
    store.save(&state).unwrap();
    assert_eq!(load(tmp.path()).unwrap(), state);
}

#[test]
fn a_legacy_state_file_is_imported_and_left_in_place() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("memory")).unwrap();
    let legacy = crate::memory::state_store::legacy_path(tmp.path(), "layout_migration");
    let state = MigrationState {
        phase: Phase::Paused,
        copied: 4,
        ..MigrationState::default()
    };
    std::fs::write(&legacy, serde_json::to_vec_pretty(&state).unwrap()).unwrap();
    assert_eq!(load(tmp.path()).unwrap(), state);
    assert!(legacy.exists());
}

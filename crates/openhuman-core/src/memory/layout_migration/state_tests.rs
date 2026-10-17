use tinymemory_tools::layout_migration::{Failure, Phase};

use super::*;

#[test]
fn a_missing_file_is_a_fresh_state() {
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(load(tmp.path()).unwrap(), MigrationState::default());
}

#[test]
fn a_saved_state_loads_back_and_leaves_no_temporary_file() {
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
    assert!(!path(tmp.path()).with_extension("json.tmp").exists());
}

#[test]
fn a_damaged_file_is_an_error_not_a_fresh_start() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("memory")).unwrap();
    std::fs::write(path(tmp.path()), b"{ not json").unwrap();
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

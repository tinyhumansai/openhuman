use super::*;
use tempfile::TempDir;

#[test]
fn a_second_migrator_is_refused_while_the_first_holds_the_lock() {
    let dir = TempDir::new().unwrap();
    let first = Lock::acquire(dir.path()).unwrap();
    let second = Lock::acquire(dir.path());
    assert!(matches!(second, Err(LayoutError::Locked(_))), "{second:?}");
    drop(first);
    Lock::acquire(dir.path()).expect("the lock is free once the holder drops it");
}

#[test]
fn state_round_trips_and_records_each_step_once() {
    let dir = TempDir::new().unwrap();
    assert!(LayoutState::load(dir.path()).unwrap().is_none());
    let mut state = LayoutState::new("u/1 weird", 1_700_000_000_000);
    assert!(
        state.archive.starts_with("u_1_weird-2023"),
        "{}",
        state.archive
    );
    assert!(state.archive.ends_with(".tar.zst"));
    state.finish(Step::Import, dir.path()).unwrap();
    state.finish(Step::Import, dir.path()).unwrap();
    let loaded = LayoutState::load(dir.path()).unwrap().unwrap();
    assert_eq!(loaded.done, vec![Step::Import]);
    assert!(loaded.has(Step::Import) && !loaded.has(Step::Migrate));
    LayoutState::clear(dir.path()).unwrap();
    LayoutState::clear(dir.path()).unwrap();
    assert!(LayoutState::load(dir.path()).unwrap().is_none());
}

#[test]
fn a_damaged_state_file_is_an_error_not_a_fresh_start() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join(STATE_FILE), "{ nope").unwrap();
    assert!(LayoutState::load(dir.path()).is_err());
}

#[test]
fn steps_are_ordered() {
    let mut sorted = Step::ALL;
    sorted.sort();
    assert_eq!(sorted, Step::ALL);
}

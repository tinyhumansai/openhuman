//! Tests for gateway persistence.
//!
//! Every case points `OPENHUMAN_WORKSPACE` at a temporary directory, so no test
//! touches a real user's records. Cargo runs unit tests as threads in one
//! process and the variable is process-wide, so they take a lock rather than
//! racing each other's workspace.

use std::sync::{Mutex, MutexGuard, PoisonError};

use super::store;
use super::types::{Confinement, Gateway, GatewaySpec, Reach, DESKTOP_ID};

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// A temporary workspace, restored when the guard drops.
struct Workspace {
    _lock: MutexGuard<'static, ()>,
    _dir: tempfile::TempDir,
    prior: Option<String>,
}

impl Drop for Workspace {
    fn drop(&mut self) {
        match self.prior.take() {
            Some(value) => std::env::set_var("OPENHUMAN_WORKSPACE", value),
            None => std::env::remove_var("OPENHUMAN_WORKSPACE"),
        }
    }
}

fn workspace() -> Workspace {
    let lock = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let dir = tempfile::TempDir::new().expect("a temporary directory");
    let prior = std::env::var("OPENHUMAN_WORKSPACE").ok();
    std::env::set_var("OPENHUMAN_WORKSPACE", dir.path());
    Workspace {
        _lock: lock,
        _dir: dir,
        prior,
    }
}

fn gateway(id: &str) -> Gateway {
    Gateway {
        id: id.to_owned(),
        label: "Build server".to_owned(),
        spec: GatewaySpec::Box {
            reach: Reach::Local,
            confinement: Confinement::Docker {
                image: "openhuman-core:latest".to_owned(),
            },
            env: Default::default(),
        },
    }
}

#[test]
fn the_desktop_gateway_is_listed_before_anything_is_saved() {
    // There is always a working answer, because the core in this process is
    // always reachable. "No gateways configured" is not a state that exists.
    let _workspace = workspace();

    let listed = store::list();

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, DESKTOP_ID);
}

#[test]
fn a_saved_gateway_survives_and_is_listed_after_the_desktop_one() {
    let _workspace = workspace();

    store::save(gateway("builder")).expect("saved");

    let listed = store::list();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, DESKTOP_ID);
    assert_eq!(listed[1].id, "builder");
    assert_eq!(
        store::get("builder").map(|g| g.label),
        Some("Build server".to_owned())
    );
}

#[test]
fn saving_an_existing_id_replaces_it_rather_than_duplicating() {
    let _workspace = workspace();
    store::save(gateway("builder")).expect("saved");

    let mut renamed = gateway("builder");
    renamed.label = "Renamed".to_owned();
    store::save(renamed).expect("saved again");

    assert_eq!(store::list().len(), 2);
    assert_eq!(
        store::get("builder").map(|g| g.label),
        Some("Renamed".to_owned())
    );
}

#[test]
fn the_desktop_id_cannot_be_taken_over() {
    // A stored record under that id would shadow the one gateway guaranteed to
    // work, which is the only way to make the app unreachable from its own UI.
    let _workspace = workspace();

    let mut impostor = gateway(DESKTOP_ID);
    impostor.label = "Not really".to_owned();

    assert!(store::save(impostor).is_err());
    assert!(store::delete(DESKTOP_ID).is_err());
    assert_eq!(
        store::get(DESKTOP_ID).map(|g| g.spec),
        Some(GatewaySpec::Desktop)
    );
}

#[test]
fn a_record_already_on_disk_under_the_desktop_id_is_ignored() {
    // Belt and braces: `save` refuses it, but a hand-edited file could still
    // contain one, and it must not win over the built-in.
    let _workspace = workspace();
    let path = crate::file_logging::resolve_data_dir().join("gateways.json");
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(
        &path,
        serde_json::to_string(&serde_json::json!({
            "gateways": [{ "id": DESKTOP_ID, "label": "Impostor", "spec": { "kind": "desktop" } }]
        }))
        .expect("json"),
    )
    .expect("write");

    let listed = store::list();

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].label, Gateway::desktop().label);
}

#[test]
fn deleting_something_that_was_never_saved_succeeds() {
    // The caller wanted it gone, and it is.
    let _workspace = workspace();

    assert!(store::delete("never-existed").is_ok());
}

#[test]
fn an_unreadable_file_degrades_to_the_desktop_gateway_rather_than_failing() {
    // Refusing to list anything would strand the user with no way back to a
    // gateway that works.
    let _workspace = workspace();
    let path = crate::file_logging::resolve_data_dir().join("gateways.json");
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(&path, "{ this is not json").expect("write");

    let listed = store::list();

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, DESKTOP_ID);
}

#[test]
fn a_gateway_without_an_id_is_refused() {
    let _workspace = workspace();

    assert!(store::save(gateway("   ")).is_err());
}

#[test]
fn a_corrupt_file_blocks_save_instead_of_being_overwritten() {
    // A malformed `gateways.json` must not be silently replaced by an empty
    // state plus the new record, which would erase every previously saved
    // gateway after a transient read failure.
    let _workspace = workspace();
    let path = crate::file_logging::resolve_data_dir().join("gateways.json");
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(&path, "{ this is not json").expect("write");

    let result = store::save(gateway("build"));

    assert!(result.is_err());
    // The corrupt bytes are untouched — nothing was written over them.
    let raw = std::fs::read_to_string(&path).expect("read raw");
    assert_eq!(raw, "{ this is not json");
}

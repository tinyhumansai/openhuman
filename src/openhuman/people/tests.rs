//! Cross-file integration tests for the people domain.

use std::sync::Arc;

use chrono::Utc;

use crate::openhuman::people::address_book;
use crate::openhuman::people::resolver::HandleResolver;
use crate::openhuman::people::store::PeopleStore;
use crate::openhuman::people::types::{Handle, PersonId};

#[tokio::test]
async fn resolver_and_store_cooperate_across_handle_kinds() {
    let s = PeopleStore::open_in_memory().unwrap();
    let r = HandleResolver::new(&s);

    // Email mints.
    let id = r
        .resolve_or_create(&Handle::Email("a@b.c".into()))
        .await
        .unwrap();
    // iMessage handle linked to same person.
    let id2 = r
        .link(
            &Handle::Email("a@b.c".into()),
            Handle::IMessage("+15551234".into()),
        )
        .await
        .unwrap();
    assert_eq!(id, id2);

    // Resolving by the linked iMessage handle returns the same id.
    let via_imsg = r
        .resolve(&Handle::IMessage("+15551234".into()))
        .await
        .unwrap();
    assert_eq!(via_imsg, Some(id));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn address_book_is_empty_on_non_mac() {
    assert!(address_book::read().unwrap().is_empty());
}

/// Verify that the schema exposes four controllers now that
/// `refresh_address_book` is wired up.
#[test]
fn schema_exposes_four_controllers() {
    use crate::openhuman::people::schemas;
    let names: Vec<_> = schemas::all_controller_schemas()
        .into_iter()
        .map(|s| s.function)
        .collect();
    assert!(
        names.contains(&"refresh_address_book"),
        "missing refresh_address_book: {names:?}"
    );
    assert_eq!(names.len(), 4);
}

/// Regression for Sentry TAURI-RUST-8NM (store never seeded → `get()` always
/// errored) and its #4378 follow-up (store stayed bound to the pre-login
/// workspace after an active-user switch). Verify `init_from_workspace` seeds
/// the global + creates the on-disk db, is an idempotent no-op for the same
/// workspace, and **rebinds** to a different workspace like `memory::global`.
///
/// Serialised (not `#[tokio::test]` parallel) because it mutates the
/// process-global store slot other people tests may observe via `get()`.
#[test]
fn init_from_workspace_seeds_and_rebinds_global_store() {
    use crate::openhuman::people::store;

    let ws_a = tempfile::tempdir().unwrap();
    let store_a = store::init_from_workspace(ws_a.path()).unwrap();
    assert!(
        ws_a.path().join("people").join("people.db").exists(),
        "seed must create <workspace>/people/people.db"
    );

    // Previously-dead global is now reachable — the 8NM fix.
    let via_global = store::get().expect("people store reachable after seed");
    assert!(Arc::ptr_eq(&store_a, &via_global));

    // Same workspace → idempotent no-op, returns the same instance.
    let again = store::init_from_workspace(ws_a.path()).unwrap();
    assert!(Arc::ptr_eq(&store_a, &again));

    // Different workspace (active-user switch) → rebind to a new store. #4378.
    let ws_b = tempfile::tempdir().unwrap();
    let store_b = store::init_from_workspace(ws_b.path()).unwrap();
    assert!(
        !Arc::ptr_eq(&store_a, &store_b),
        "a new workspace must rebind to a fresh store, not reuse the old one"
    );
    let after_switch = store::get().expect("people store reachable after rebind");
    assert!(
        Arc::ptr_eq(&store_b, &after_switch),
        "get() must return the rebound (workspace B) store after a switch"
    );
}

#[test]
fn person_id_uuid_format() {
    let id = PersonId::new();
    // Round-trips through a string.
    let s = id.to_string();
    let parsed: uuid::Uuid = s.parse().unwrap();
    assert_eq!(parsed, id.0);
    let _now = Utc::now();
}

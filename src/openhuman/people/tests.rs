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

/// Regression for Sentry TAURI-RUST-8NM: the process-global people store was
/// never seeded at boot, so `store::get()` (and every `people_*` tool /
/// controller) always failed with "people store not initialised". Boot now
/// calls `init_from_workspace`; verify it seeds the global, creates the on-disk
/// db, and is idempotent.
#[tokio::test]
async fn init_from_workspace_seeds_global_store() {
    use crate::openhuman::people::store;

    let tmp = tempfile::tempdir().unwrap();
    let store = store::init_from_workspace(tmp.path()).await.unwrap();
    assert!(
        tmp.path().join("people").join("people.db").exists(),
        "boot seed must create <workspace>/people/people.db"
    );

    // The previously-dead global is now reachable — this is the fix.
    let via_global = store::get().expect("people store reachable after boot seed");
    assert!(Arc::ptr_eq(&store, &via_global));

    // Idempotent: a second seed returns the same instance, never errors.
    let again = store::init_from_workspace(tmp.path()).await.unwrap();
    assert!(Arc::ptr_eq(&store, &again));
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

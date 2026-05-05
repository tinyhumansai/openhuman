//! Cross-file integration tests for the people domain.

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

#[test]
fn person_id_uuid_format() {
    let id = PersonId::new();
    // Round-trips through a string.
    let s = id.to_string();
    let parsed: uuid::Uuid = s.parse().unwrap();
    assert_eq!(parsed, id.0);
    let _now = Utc::now();
}

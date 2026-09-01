/// The `contacts` gate must reach the engine, not stop at this crate.
///
/// The macOS address-book reader lives in the memory engine, several crates
/// below this one, behind `#[cfg(all(target_os = "macos", feature =
/// "contacts"))]`. This crate's `contacts` feature once enabled four
/// `objc2` crates *locally* — none of which any file in `src/` names — and
/// never forwarded, so the reader was always compiled out. Nothing failed:
/// `refresh_address_book` returned success having seeded zero contacts, and
/// the only visible symptom was an address book that stayed empty.
///
/// So this asserts the property that was actually missing — that turning
/// the feature on *here* changes what the reader does *there*. A build with
/// `contacts` on, on macOS, must reach the real `CNContactStore` arm; the
/// stub returns `Ok(vec![])` unconditionally, and the real arm cannot,
/// because it can fail on permission.
///
/// Deliberately not a `cfg!(feature = ...)` self-assertion: that would pass
/// while the forward is broken, which is the entire bug.
///
/// Names the engine crate directly rather than through `super::*`, which was
/// the glob re-export until #5560 deleted it as unused. This is the one
/// remaining reader, it is `#[cfg(test)]`, and a test reference does not link
/// the crate into the shipped binary — so the gate keeps testing the forward
/// without the production edge it used to travel over.
#[test]
#[cfg(all(target_os = "macos", feature = "contacts"))]
fn contacts_feature_reaches_the_engine_reader() {
    use tinycortex::memory::people::address_book::{
        AddressBookError, ContactsSource, SystemContactsSource,
    };

    // The stub arm returns Ok(vec![]) and can never report a permission
    // failure. Reaching a `PermissionDenied` — or real contacts — proves the
    // macOS arm compiled in. On a CI box with no Contacts authorisation the
    // permission error is the expected outcome.
    match SystemContactsSource.fetch_contacts() {
        Err(AddressBookError::PermissionDenied) => {}
        Ok(_) => {}
        Err(other) => panic!("address book read failed unexpectedly: {other:?}"),
    }
}

/// Off macOS the gate is a documented no-op, and the stub is correct.
#[test]
#[cfg(not(target_os = "macos"))]
fn contacts_gate_is_a_no_op_off_macos() {
    use tinycortex::memory::people::address_book::{ContactsSource, SystemContactsSource};

    assert_eq!(
        SystemContactsSource
            .fetch_contacts()
            .expect("stub never fails"),
        vec![],
        "off macOS the reader must be the empty stub"
    );
}

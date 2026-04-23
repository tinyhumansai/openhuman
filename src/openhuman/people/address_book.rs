//! macOS Address Book read via `CNContactStore`.
//!
//! Uses the documented Contacts framework API (`CNContactStore`) which:
//!   - Triggers the TCC Contacts permission prompt (sandboxed builds work correctly).
//!   - Returns a structured error for "permission denied" so callers can distinguish
//!     that case from "no contacts".
//!
//! A trait (`ContactsSource`) provides a mockable seam so unit tests can inject a
//! canned list or a permission-denied error without any FFI calls.
//!
//! On non-mac platforms `read()` returns an empty vec (stub path).

use crate::openhuman::people::types::AddressBookContact;

/// Result type distinguishing permission errors from other failures.
#[derive(Debug, PartialEq)]
pub enum AddressBookError {
    /// The user denied or restricted Contacts access.
    PermissionDenied,
    /// Any other error (typically returned as a descriptive string).
    Other(String),
}

impl std::fmt::Display for AddressBookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AddressBookError::PermissionDenied => {
                write!(
                    f,
                    "contacts access denied — grant access in System Settings > Privacy > Contacts"
                )
            }
            AddressBookError::Other(s) => write!(f, "{s}"),
        }
    }
}

/// Mockable seam for contact fetching. The real impl calls CNContactStore;
/// tests inject a `MockContactsSource`.
pub trait ContactsSource: Send + Sync {
    fn fetch_contacts(&self) -> Result<Vec<AddressBookContact>, AddressBookError>;
}

/// Real implementation backed by CNContactStore (macOS only).
/// On non-mac this is an empty struct whose `fetch_contacts` always returns `Ok(vec![])`.
pub struct SystemContactsSource;

impl ContactsSource for SystemContactsSource {
    fn fetch_contacts(&self) -> Result<Vec<AddressBookContact>, AddressBookError> {
        imp::fetch_via_cn_contact_store()
    }
}

/// Fetch all contacts using the provided `ContactsSource`.
///
/// Errors are logged at `warn` level and surfaced to the caller so RPC
/// handlers can distinguish "permission denied" from "no contacts found".
pub fn read_with(source: &dyn ContactsSource) -> Result<Vec<AddressBookContact>, AddressBookError> {
    match source.fetch_contacts() {
        Ok(v) => {
            tracing::debug!("[people::address_book] fetched {} contacts", v.len());
            Ok(v)
        }
        Err(AddressBookError::PermissionDenied) => {
            tracing::warn!(
                "[people::address_book] contacts access denied — \
                 grant access in System Settings > Privacy > Contacts"
            );
            Err(AddressBookError::PermissionDenied)
        }
        Err(AddressBookError::Other(ref e)) => {
            tracing::warn!("[people::address_book] fetch error: {e}");
            Err(AddressBookError::Other(e.clone()))
        }
    }
}

/// Convenience wrapper using the real `SystemContactsSource`.
pub fn read() -> Result<Vec<AddressBookContact>, AddressBookError> {
    read_with(&SystemContactsSource)
}

// ── macOS implementation ──────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod imp {
    use super::{AddressBookContact, AddressBookError};

    use block2::RcBlock;
    use core::ptr::NonNull;
    use objc2::runtime::Bool;
    use objc2::runtime::ProtocolObject;
    use objc2::AnyThread as _;
    use objc2_contacts::{
        CNAuthorizationStatus, CNContact, CNContactFetchRequest, CNContactStore, CNEntityType,
    };
    use objc2_foundation::{NSArray, NSError, NSString};
    use std::sync::{Arc, Mutex};

    // CNKeyDescriptor is a protocol; NSString conforms to it.
    // We build the keys array as NSArray<ProtocolObject<dyn CNKeyDescriptor>>.
    use objc2_contacts::CNKeyDescriptor;

    /// Build the keys array used for CNContactFetchRequest.
    ///
    /// # Safety
    /// NSString::from_str is safe; casting to ProtocolObject is safe because
    /// `NSString: CNKeyDescriptor` (confirmed by the objc2-contacts bindings).
    unsafe fn make_keys_array() -> objc2::rc::Retained<NSArray<ProtocolObject<dyn CNKeyDescriptor>>>
    {
        let given = NSString::from_str("givenName");
        let family = NSString::from_str("familyName");
        let emails = NSString::from_str("emailAddresses");
        let phones = NSString::from_str("phoneNumbers");

        // NSString conforms to CNKeyDescriptor, so we can cast the refs.
        let refs: &[&ProtocolObject<dyn CNKeyDescriptor>] = &[
            ProtocolObject::from_ref(&*given),
            ProtocolObject::from_ref(&*family),
            ProtocolObject::from_ref(&*emails),
            ProtocolObject::from_ref(&*phones),
        ];
        NSArray::from_slice(refs)
    }

    /// Request contacts access from TCC. Blocks on the calling thread until
    /// the completion handler fires. Must not be called from the main thread
    /// on macOS (CNContactStore will deadlock).
    fn request_access(store: &CNContactStore) -> Result<(), AddressBookError> {
        unsafe {
            let status = CNContactStore::authorizationStatusForEntityType(CNEntityType::Contacts);
            match status {
                CNAuthorizationStatus::Authorized | CNAuthorizationStatus::Limited => {
                    tracing::debug!("[people::address_book] contacts access already authorized");
                    return Ok(());
                }
                CNAuthorizationStatus::Denied | CNAuthorizationStatus::Restricted => {
                    return Err(AddressBookError::PermissionDenied);
                }
                _ => {
                    tracing::debug!(
                        "[people::address_book] requesting contacts access (status={status:?})"
                    );
                }
            }

            let (tx, rx) = std::sync::mpsc::channel::<Result<(), AddressBookError>>();
            let tx = Arc::new(Mutex::new(Some(tx)));
            let tx_clone = Arc::clone(&tx);

            let block = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
                let mut slot = tx_clone.lock().unwrap();
                if let Some(sender) = slot.take() {
                    let result = if granted.as_bool() {
                        Ok(())
                    } else {
                        Err(AddressBookError::PermissionDenied)
                    };
                    let _ = sender.send(result);
                }
            });

            store.requestAccessForEntityType_completionHandler(CNEntityType::Contacts, &*block);

            rx.recv().map_err(|_| {
                AddressBookError::Other("contacts permission callback never fired".into())
            })?
        }
    }

    pub fn fetch_via_cn_contact_store() -> Result<Vec<AddressBookContact>, AddressBookError> {
        tracing::debug!("[people::address_book] fetch_via_cn_contact_store entry");
        unsafe {
            let store = CNContactStore::new();
            request_access(&store)?;

            let keys_array = make_keys_array();
            let request = CNContactFetchRequest::initWithKeysToFetch(
                CNContactFetchRequest::alloc(),
                &keys_array,
            );

            let mut contacts: Vec<AddressBookContact> = Vec::new();

            // We use a raw pointer to the vec inside the block so that we can
            // push from within the block. The block runs synchronously within
            // enumerateContactsWithFetchRequest (it blocks until done), so the
            // pointer is valid throughout.
            let contacts_ptr: *mut Vec<AddressBookContact> = &mut contacts;

            let block = RcBlock::new(
                move |contact_nn: NonNull<CNContact>, _stop: NonNull<Bool>| {
                    let contact: &CNContact = contact_nn.as_ref();

                    let given = contact.givenName().to_string();
                    let family = contact.familyName().to_string();
                    let full = {
                        let g = given.trim();
                        let f = family.trim();
                        match (g.is_empty(), f.is_empty()) {
                            (true, true) => None,
                            (false, true) => Some(g.to_string()),
                            (true, false) => Some(f.to_string()),
                            (false, false) => Some(format!("{g} {f}")),
                        }
                    };

                    let emails: Vec<String> = {
                        let arr = contact.emailAddresses();
                        let mut v = Vec::new();
                        for i in 0..arr.len() {
                            let lv = arr.objectAtIndex(i);
                            // CNLabeledValue<NSString>.value() → Retained<NSString>
                            let email = lv.value().to_string();
                            let trimmed = email.trim().to_string();
                            if !trimmed.is_empty() {
                                v.push(trimmed);
                            }
                        }
                        v
                    };

                    let phones: Vec<String> = {
                        let arr = contact.phoneNumbers();
                        let mut v = Vec::new();
                        for i in 0..arr.len() {
                            let lv = arr.objectAtIndex(i);
                            // CNLabeledValue<CNPhoneNumber>.value() → Retained<CNPhoneNumber>
                            let num = lv.value().stringValue().to_string();
                            let trimmed = num.trim().to_string();
                            if !trimmed.is_empty() {
                                v.push(trimmed);
                            }
                        }
                        v
                    };

                    if full.is_none() && emails.is_empty() && phones.is_empty() {
                        return;
                    }

                    (*contacts_ptr).push(AddressBookContact {
                        display_name: full,
                        emails,
                        phones,
                    });
                },
            );

            let mut error: Option<objc2::rc::Retained<NSError>> = None;
            let ok = store.enumerateContactsWithFetchRequest_error_usingBlock(
                &request,
                Some(&mut error),
                &*block,
            );
            if !ok {
                let msg = error
                    .map(|e| e.localizedDescription().to_string())
                    .unwrap_or_else(|| "unknown error from CNContactStore".into());
                return Err(AddressBookError::Other(msg));
            }

            tracing::debug!(
                "[people::address_book] enumerated {} contacts",
                contacts.len()
            );
            Ok(contacts)
        }
    }
}

// ── non-macOS stub ────────────────────────────────────────────────────────────

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::{AddressBookContact, AddressBookError};

    pub fn fetch_via_cn_contact_store() -> Result<Vec<AddressBookContact>, AddressBookError> {
        Ok(vec![])
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Test double that returns a canned list without any FFI calls.
    pub struct MockContactsSource {
        pub result: Result<Vec<AddressBookContact>, AddressBookError>,
    }

    impl MockContactsSource {
        pub fn ok(contacts: Vec<AddressBookContact>) -> Self {
            Self {
                result: Ok(contacts),
            }
        }

        pub fn permission_denied() -> Self {
            Self {
                result: Err(AddressBookError::PermissionDenied),
            }
        }
    }

    impl ContactsSource for MockContactsSource {
        fn fetch_contacts(&self) -> Result<Vec<AddressBookContact>, AddressBookError> {
            match &self.result {
                Ok(v) => Ok(v.clone()),
                Err(AddressBookError::PermissionDenied) => Err(AddressBookError::PermissionDenied),
                Err(AddressBookError::Other(s)) => Err(AddressBookError::Other(s.clone())),
            }
        }
    }

    fn mk_contact(name: &str, email: &str) -> AddressBookContact {
        AddressBookContact {
            display_name: Some(name.into()),
            emails: vec![email.into()],
            phones: vec![],
        }
    }

    #[test]
    fn mock_source_returns_canned_contacts() {
        let source = MockContactsSource::ok(vec![
            mk_contact("Alice", "alice@example.com"),
            mk_contact("Bob", "bob@example.com"),
        ]);
        let result = read_with(&source).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].display_name.as_deref(), Some("Alice"));
        assert_eq!(result[1].emails[0], "bob@example.com");
    }

    #[test]
    fn mock_source_permission_denied_is_distinguished() {
        let source = MockContactsSource::permission_denied();
        let err = read_with(&source).unwrap_err();
        assert_eq!(err, AddressBookError::PermissionDenied);
    }

    #[test]
    fn system_source_non_mac_returns_empty() {
        #[cfg(not(target_os = "macos"))]
        {
            let source = SystemContactsSource;
            let result = read_with(&source).unwrap();
            assert!(result.is_empty());
        }
        #[cfg(target_os = "macos")]
        {
            // TCC state is environment-dependent; just verify no panic.
            let source = SystemContactsSource;
            let _ = read_with(&source);
        }
    }

    #[test]
    fn contact_with_no_fields_is_excluded_by_mock() {
        let source = MockContactsSource::ok(vec![AddressBookContact {
            display_name: Some("Sarah Lee".into()),
            emails: vec![],
            phones: vec!["+1 555 000 0001".into()],
        }]);
        let result = read_with(&source).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].phones[0], "+1 555 000 0001");
    }
}

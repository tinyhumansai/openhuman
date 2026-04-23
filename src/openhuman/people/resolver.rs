//! HandleResolver — deterministic mapping (Handle) → PersonId.
//!
//! Given the same store contents, resolving the same handle twice returns
//! the same `PersonId`. If the handle is unknown and `create_if_missing`
//! is set, the resolver mints a new `PersonId`, inserts a `Person` skeleton
//! with the handle attached, and returns the new id.
//!
//! `seed_from_address_book` wires the `address_book` read path into the
//! resolver so that contacts from the system address book are pre-populated
//! as `Person` rows (and their handles are registered for future resolution).

use chrono::Utc;

use crate::openhuman::people::address_book::{self, AddressBookError, ContactsSource};
use crate::openhuman::people::store::PeopleStore;
use crate::openhuman::people::types::{Handle, Person, PersonId};

pub struct HandleResolver<'a> {
    store: &'a PeopleStore,
}

impl<'a> HandleResolver<'a> {
    pub fn new(store: &'a PeopleStore) -> Self {
        Self { store }
    }

    /// Look up the person for a handle. Returns `None` if unknown.
    pub async fn resolve(&self, handle: &Handle) -> Result<Option<PersonId>, String> {
        self.store
            .lookup(handle)
            .await
            .map_err(|e| format!("lookup: {e}"))
    }

    /// Look up or mint. Display-name / email fields on the newly-minted
    /// `Person` are populated from the handle itself so the UI has
    /// something to render before any enrichment runs.
    pub async fn resolve_or_create(&self, handle: &Handle) -> Result<PersonId, String> {
        let canonical = handle.canonicalize();
        if let Some(id) = self
            .store
            .lookup(&canonical)
            .await
            .map_err(|e| format!("lookup: {e}"))?
        {
            return Ok(id);
        }
        let id = PersonId::new();
        let (display_name, primary_email, primary_phone) = match &canonical {
            Handle::DisplayName(s) => (Some(s.clone()), None, None),
            Handle::Email(s) => (None, Some(s.clone()), None),
            Handle::IMessage(s) => {
                if s.contains('@') {
                    (None, Some(s.clone()), None)
                } else {
                    (None, None, Some(s.clone()))
                }
            }
        };
        let now = Utc::now();
        let person = Person {
            id,
            display_name,
            primary_email,
            primary_phone,
            handles: vec![canonical.clone()],
            created_at: now,
            updated_at: now,
        };
        self.store
            .insert_person(&person, &[canonical])
            .await
            .map_err(|e| format!("insert_person: {e}"))?;
        Ok(id)
    }

    /// Merge: attach `other` as an alias on the person `primary` resolves to.
    /// Useful for the sync path that learns "this email and this phone
    /// belong to the same contact".
    pub async fn link(&self, primary: &Handle, other: Handle) -> Result<PersonId, String> {
        let pid = self.resolve_or_create(primary).await?;
        self.store
            .add_alias(pid, other)
            .await
            .map_err(|e| format!("add_alias: {e}"))?;
        Ok(pid)
    }

    /// Seed the people store from the system address book.
    ///
    /// For each contact returned by `source`:
    ///   - Pick the first email or phone as the "primary" handle and look it
    ///     up or mint a `PersonId`.
    ///   - Link any additional emails / phones as aliases on the same person.
    ///   - If only a display name is present, mint via display name.
    ///
    /// Contacts that produce no handles at all are skipped. This is
    /// idempotent: re-running on the same contact list is a no-op because
    ///`lookup` finds existing handle rows.
    ///
    /// Returns `(seeded, skipped)` counts, and propagates `AddressBookError`
    /// to let callers distinguish permission-denied from other failures.
    pub async fn seed_from_address_book(
        &self,
        source: &dyn ContactsSource,
    ) -> Result<(usize, usize), AddressBookError> {
        let contacts = address_book::read_with(source)?;
        let mut seeded = 0usize;
        let mut skipped = 0usize;

        for c in contacts {
            // Build a flat list of all handles for this contact.
            let mut handles: Vec<Handle> = Vec::new();
            for email in &c.emails {
                let trimmed = email.trim();
                if !trimmed.is_empty() {
                    handles.push(Handle::Email(trimmed.to_string()));
                }
            }
            for phone in &c.phones {
                let trimmed = phone.trim();
                if !trimmed.is_empty() {
                    handles.push(Handle::IMessage(trimmed.to_string()));
                }
            }
            if let Some(ref name) = c.display_name {
                let trimmed = name.trim();
                if !trimmed.is_empty() {
                    handles.push(Handle::DisplayName(trimmed.to_string()));
                }
            }

            if handles.is_empty() {
                skipped += 1;
                continue;
            }

            // The "primary" handle is the first email if present, otherwise
            // the first phone, otherwise the display name. This gives the
            // most stable link target for future interactions.
            let primary = handles[0].clone();

            // mint or look up the primary handle
            match self.resolve_or_create(&primary).await {
                Err(e) => {
                    tracing::warn!(
                        "[people::resolver] seed_from_address_book: failed to upsert primary handle {:?}: {e}",
                        primary.as_key()
                    );
                    skipped += 1;
                    continue;
                }
                Ok(pid) => {
                    // link all additional handles as aliases
                    for alias in handles.into_iter().skip(1) {
                        if let Err(e) = self.store.add_alias(pid, alias.canonicalize()).await {
                            tracing::warn!(
                                "[people::resolver] seed_from_address_book: add_alias failed: {e}"
                            );
                        }
                    }
                    seeded += 1;
                }
            }
        }

        tracing::debug!(
            "[people::resolver] seed_from_address_book done: seeded={seeded} skipped={skipped}"
        );
        Ok((seeded, skipped))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::people::address_book::tests::MockContactsSource;
    use crate::openhuman::people::types::AddressBookContact;

    #[tokio::test]
    async fn resolve_returns_none_for_unknown_handle() {
        let s = PeopleStore::open_in_memory().unwrap();
        let r = HandleResolver::new(&s);
        let got = r.resolve(&Handle::Email("x@y.z".into())).await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn resolve_or_create_is_deterministic_across_case_and_whitespace() {
        let s = PeopleStore::open_in_memory().unwrap();
        let r = HandleResolver::new(&s);
        let a = r
            .resolve_or_create(&Handle::Email("Sarah@Example.COM".into()))
            .await
            .unwrap();
        let b = r
            .resolve_or_create(&Handle::Email("  sarah@example.com ".into()))
            .await
            .unwrap();
        assert_eq!(a, b, "canonicalization must collapse case+whitespace");
    }

    #[tokio::test]
    async fn same_email_different_display_name_resolve_same_id() {
        let s = PeopleStore::open_in_memory().unwrap();
        let r = HandleResolver::new(&s);
        let via_email = r
            .resolve_or_create(&Handle::Email("a@b.c".into()))
            .await
            .unwrap();
        // Linking a display name to the same email must not mint a second id.
        let via_linked = r
            .link(
                &Handle::Email("a@b.c".into()),
                Handle::DisplayName("Alice".into()),
            )
            .await
            .unwrap();
        assert_eq!(via_email, via_linked);
        // And now resolving the display name returns the same id.
        let via_name = r
            .resolve(&Handle::DisplayName("Alice".into()))
            .await
            .unwrap();
        assert_eq!(via_name, Some(via_email));
    }

    #[tokio::test]
    async fn distinct_handles_without_linking_produce_distinct_ids() {
        let s = PeopleStore::open_in_memory().unwrap();
        let r = HandleResolver::new(&s);
        let a = r
            .resolve_or_create(&Handle::Email("a@b.c".into()))
            .await
            .unwrap();
        let b = r
            .resolve_or_create(&Handle::Email("x@y.z".into()))
            .await
            .unwrap();
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn seed_from_address_book_populates_store() {
        let s = PeopleStore::open_in_memory().unwrap();
        let r = HandleResolver::new(&s);

        let source = MockContactsSource::ok(vec![
            AddressBookContact {
                display_name: Some("Alice Smith".into()),
                emails: vec!["alice@example.com".into()],
                phones: vec!["+1 555 000 0001".into()],
            },
            AddressBookContact {
                display_name: Some("Bob Jones".into()),
                emails: vec!["bob@example.com".into()],
                phones: vec![],
            },
        ]);

        let (seeded, skipped) = r.seed_from_address_book(&source).await.unwrap();
        assert_eq!(seeded, 2, "both contacts should be seeded");
        assert_eq!(skipped, 0);

        // Alice is resolvable by email
        let alice_id = r
            .resolve(&Handle::Email("alice@example.com".into()))
            .await
            .unwrap();
        assert!(alice_id.is_some(), "alice must be resolvable after seed");

        // Alice is also resolvable by phone (linked as alias)
        let alice_via_phone = r
            .resolve(&Handle::IMessage("+1 555 000 0001".into()))
            .await
            .unwrap();
        assert_eq!(
            alice_id, alice_via_phone,
            "email and phone must resolve to same person"
        );

        // Bob is resolvable
        let bob_id = r
            .resolve(&Handle::Email("bob@example.com".into()))
            .await
            .unwrap();
        assert!(bob_id.is_some());
        assert_ne!(alice_id, bob_id, "distinct contacts must have distinct ids");
    }

    #[tokio::test]
    async fn seed_from_address_book_permission_denied_is_propagated() {
        let s = PeopleStore::open_in_memory().unwrap();
        let r = HandleResolver::new(&s);

        let source = MockContactsSource::permission_denied();
        let err = r.seed_from_address_book(&source).await.unwrap_err();
        assert_eq!(err, AddressBookError::PermissionDenied);

        // Store must still be empty — no partial writes.
        let people = s.list().await.unwrap();
        assert!(
            people.is_empty(),
            "no people should be inserted on permission denied"
        );
    }

    #[tokio::test]
    async fn seed_is_idempotent() {
        let s = PeopleStore::open_in_memory().unwrap();
        let r = HandleResolver::new(&s);

        let source = MockContactsSource::ok(vec![AddressBookContact {
            display_name: Some("Carol".into()),
            emails: vec!["carol@example.com".into()],
            phones: vec![],
        }]);

        let (s1, _) = r.seed_from_address_book(&source).await.unwrap();
        let (s2, _) = r.seed_from_address_book(&source).await.unwrap();
        assert_eq!(s1, 1);
        assert_eq!(s2, 1, "second seed call should still report 1 (upsert)");

        // Only one person in store.
        let people = s.list().await.unwrap();
        assert_eq!(people.len(), 1, "idempotent — must not duplicate");
    }

    #[tokio::test]
    async fn contact_with_only_display_name_is_seeded() {
        let s = PeopleStore::open_in_memory().unwrap();
        let r = HandleResolver::new(&s);

        let source = MockContactsSource::ok(vec![AddressBookContact {
            display_name: Some("No Email Person".into()),
            emails: vec![],
            phones: vec![],
        }]);
        let (seeded, skipped) = r.seed_from_address_book(&source).await.unwrap();
        assert_eq!(seeded, 1);
        assert_eq!(skipped, 0);
    }

    #[tokio::test]
    async fn contact_with_no_fields_is_skipped() {
        let s = PeopleStore::open_in_memory().unwrap();
        let r = HandleResolver::new(&s);

        let source = MockContactsSource::ok(vec![AddressBookContact {
            display_name: None,
            emails: vec![],
            phones: vec![],
        }]);
        let (seeded, skipped) = r.seed_from_address_book(&source).await.unwrap();
        assert_eq!(seeded, 0);
        assert_eq!(skipped, 1);
    }
}

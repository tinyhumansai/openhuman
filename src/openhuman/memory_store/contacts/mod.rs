//! Contacts storage facade.
//!
//! Contacts (the `people` domain — resolver, scorer, address book) are owned
//! by `src/openhuman/people/`. This module re-exports the storage surface so
//! `memory_store` is a single import point for ALL stored memory kinds: raw
//! content, chunks, summary trees, vectors, AND contacts.
//!
//! No data is duplicated: `PeopleStore` is the source of truth and its
//! singleton (`people::store::get`) backs every call routed through here.
//! When the user asks "what does memory_store store?", the answer is the
//! union of: chunks, content (md files), trees (Source/Global/Topic),
//! vectors, AND contacts.

pub use crate::openhuman::people::store::{get as get_store, init as init_store, PeopleStore};
pub use crate::openhuman::people::types::{
    AddressBookContact, Handle, Interaction, Person, PersonId, ScoreComponents,
};

/// Async helper: load a contact by id from the global PeopleStore. Returns
/// `Ok(None)` when no PersonId matches (and when the store hasn't been
/// initialized — callers in early-boot paths shouldn't crash).
pub async fn get_contact(person_id: PersonId) -> Option<Person> {
    match get_store() {
        Ok(s) => s.get(person_id).await.ok().flatten(),
        Err(_) => None,
    }
}

/// Async helper: list every stored contact. Returns an empty vec if the store
/// is uninitialized — matches the read-side fail-soft contract used by the
/// rest of the memory_store retrieval surface.
pub async fn list_contacts() -> Vec<Person> {
    match get_store() {
        Ok(s) => s.list().await.unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Async helper: resolve a handle to its canonical `PersonId` without
/// inserting. Used by retrieval paths that need a person id but don't want
/// to mint a new contact for an unknown handle.
pub async fn lookup_contact(handle: &Handle) -> Option<PersonId> {
    match get_store() {
        Ok(s) => s.lookup(handle).await.ok().flatten(),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn contact_facade_fail_softs_when_store_is_uninitialized() {
        let missing = get_contact(PersonId(Uuid::nil())).await;
        assert!(missing.is_none());

        let listed = list_contacts().await;
        assert!(listed.is_empty());

        let looked_up = lookup_contact(&Handle::Email("nobody@example.com".into())).await;
        assert!(looked_up.is_none());
    }
}

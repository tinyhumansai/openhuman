//! Memory entities — Obsidian-md-backed registry of people and other named
//! things in the user's world.
//!
//! Replacement for the SQLite-backed `people/` module. The data lives as
//! markdown files in the content store so the user's vault is the source
//! of truth and arbitrary tools (Obsidian itself, grep, vector search)
//! can introspect or edit it.
//!
//! ## On disk
//!
//! ```text
//! <content_root>/entities/<kind>/<canonical_id>.md
//! ```
//!
//! Each file:
//!
//! ```markdown
//! ---
//! id: <canonical id>
//! kind: person | organization | topic | email | url | hashtag | ...
//! display_name: <free-form>
//! aliases:
//!   - "<alias 1>"
//!   - "<alias 2>"
//! emails:
//!   - "<email>"
//! handles:
//!   - kind: imessage
//!     value: "+15555550100"
//! created_at: <rfc3339>
//! updated_at: <rfc3339>
//! ---
//!
//! <free-form notes; the user can edit this body in Obsidian>
//! ```
//!
//! `kind` matches [`memory_tree::score::extract::EntityKind`] verbatim so the
//! same canonical-id format the scorer emits round-trips through here.
//!
//! ## API
//!
//! - [`store::put_entity`]      — upsert by canonical id (atomic write).
//! - [`store::get_entity`]      — read by canonical id.
//! - [`store::list_entities`]   — walk a kind directory.
//! - [`store::lookup_alias`]    — find a canonical id by alias / email /
//!   handle (linear scan; fine for the order-of-magnitudes a single user
//!   accumulates).
//!
//! ## Migration from `people/`
//!
//! `people::Person` maps onto [`Entity { kind: Person, ... }`]. The handle
//! types (`IMessage`, `Email`, `DisplayName`) become entries in the
//! `handles` / `emails` / `aliases` fields. The SQLite resolver and
//! address-book code in `people/` continues to work in parallel until
//! every caller switches to this module; this is a scaffold, not a
//! cut-over.

pub mod store;
pub mod types;

pub use store::{get_entity, list_entities, lookup_alias, put_entity};
pub use types::{Entity, EntityHandle, EntityKind};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_reexports_are_constructible() {
        let handle = EntityHandle {
            kind: "slack".into(),
            value: "@alice".into(),
        };
        let mut entity = Entity::new("person:alice", EntityKind::Person);
        entity.handles.push(handle.clone());

        assert_eq!(entity.kind, EntityKind::Person);
        assert_eq!(entity.handles, vec![handle]);
    }
}

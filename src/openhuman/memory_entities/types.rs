//! Entity shape.
//!
//! One serde struct covers every kind. `kind` field discriminates; the
//! optional fields (`emails`, `handles`, `aliases`, `notes`) populate as
//! relevant.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Kinds an entity can take. Mirrors
/// [`crate::openhuman::memory::score::extract::EntityKind`] so canonical
/// ids the scorer emits round-trip through this module unchanged. Kept as
/// a local enum (not a re-export) so memory_entities stays usable
/// independently of the score module's exact internals.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Person,
    Organization,
    Topic,
    Email,
    Url,
    Handle,
    Hashtag,
    Location,
    Event,
    Product,
    Datetime,
    Technology,
    Artifact,
    Quantity,
    Misc,
}

impl EntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EntityKind::Person => "person",
            EntityKind::Organization => "organization",
            EntityKind::Topic => "topic",
            EntityKind::Email => "email",
            EntityKind::Url => "url",
            EntityKind::Handle => "handle",
            EntityKind::Hashtag => "hashtag",
            EntityKind::Location => "location",
            EntityKind::Event => "event",
            EntityKind::Product => "product",
            EntityKind::Datetime => "datetime",
            EntityKind::Technology => "technology",
            EntityKind::Artifact => "artifact",
            EntityKind::Quantity => "quantity",
            EntityKind::Misc => "misc",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "person" => Ok(Self::Person),
            "organization" => Ok(Self::Organization),
            "topic" => Ok(Self::Topic),
            "email" => Ok(Self::Email),
            "url" => Ok(Self::Url),
            "handle" => Ok(Self::Handle),
            "hashtag" => Ok(Self::Hashtag),
            "location" => Ok(Self::Location),
            "event" => Ok(Self::Event),
            "product" => Ok(Self::Product),
            "datetime" => Ok(Self::Datetime),
            "technology" => Ok(Self::Technology),
            "artifact" => Ok(Self::Artifact),
            "quantity" => Ok(Self::Quantity),
            "misc" => Ok(Self::Misc),
            other => Err(format!("unknown entity kind: {other}")),
        }
    }
}

/// A handle is an opaque label by which this entity is known to a source.
/// Generalisation of `people::Handle` — works for emails, phone numbers,
/// social handles, anything that identifies the entity in one channel
/// without being its canonical id.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntityHandle {
    /// e.g. `"imessage"`, `"slack"`, `"discord"`, `"gmail"`.
    pub kind: String,
    pub value: String,
}

/// One entity. Persisted as `<content_root>/entities/<kind>/<canonical_id>.md`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entity {
    /// Canonical id — `<kind>:<value>` (e.g. `person:alice`,
    /// `email:alice@example.com`). Stable across renames and aliases.
    pub id: String,
    pub kind: EntityKind,
    /// Free-form display name. `None` when the user hasn't named the
    /// entity yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Alternate strings the entity is known by (nicknames, old names).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    /// Email addresses associated with the entity. Pulled out of the
    /// generic `handles` for Person convenience.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emails: Vec<String>,
    /// Source-specific handles (slack, discord, imessage, …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub handles: Vec<EntityHandle>,
    /// First write timestamp.
    pub created_at: DateTime<Utc>,
    /// Last upsert timestamp.
    pub updated_at: DateTime<Utc>,
}

impl Entity {
    /// Construct a fresh entity. `id` should already be canonicalized
    /// (`<kind>:<value>`); callers are responsible for that.
    pub fn new(id: impl Into<String>, kind: EntityKind) -> Self {
        let now = Utc::now();
        Self {
            id: id.into(),
            kind,
            display_name: None,
            aliases: Vec::new(),
            emails: Vec::new(),
            handles: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn entity_kind_roundtrips() {
        for kind in [
            EntityKind::Person,
            EntityKind::Organization,
            EntityKind::Topic,
            EntityKind::Email,
            EntityKind::Url,
            EntityKind::Handle,
            EntityKind::Hashtag,
            EntityKind::Location,
            EntityKind::Event,
            EntityKind::Product,
            EntityKind::Datetime,
            EntityKind::Technology,
            EntityKind::Artifact,
            EntityKind::Quantity,
            EntityKind::Misc,
        ] {
            assert_eq!(EntityKind::parse(kind.as_str()).unwrap(), kind);
        }
    }

    #[test]
    fn entity_new_sets_empty_collections_and_timestamps() {
        let entity = Entity::new("person:alice", EntityKind::Person);
        assert_eq!(entity.id, "person:alice");
        assert_eq!(entity.kind, EntityKind::Person);
        assert!(entity.display_name.is_none());
        assert!(entity.aliases.is_empty());
        assert!(entity.emails.is_empty());
        assert!(entity.handles.is_empty());
        assert_eq!(entity.created_at, entity.updated_at);
    }

    #[test]
    fn entity_handle_and_entity_serde_roundtrip() {
        let entity = Entity {
            id: "person:alice".into(),
            kind: EntityKind::Person,
            display_name: Some("Alice".into()),
            aliases: vec!["A".into()],
            emails: vec!["alice@example.com".into()],
            handles: vec![EntityHandle {
                kind: "slack".into(),
                value: "@alice".into(),
            }],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let value = serde_json::to_value(&entity).unwrap();
        assert_eq!(value["id"], json!("person:alice"));
        assert_eq!(value["kind"], json!("person"));
        assert_eq!(value["display_name"], json!("Alice"));

        let decoded: Entity = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.id, entity.id);
        assert_eq!(decoded.kind, entity.kind);
        assert_eq!(decoded.display_name, entity.display_name);
        assert_eq!(decoded.aliases, entity.aliases);
        assert_eq!(decoded.emails, entity.emails);
        assert_eq!(decoded.handles, entity.handles);
    }
}

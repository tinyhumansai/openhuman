//! Unit tests for WhatsApp data controller schema registration.

use super::{all_controller_schemas, all_internal_controllers, all_registered_controllers};

/// The agent-facing registry must expose exactly the three read-only tools:
/// list_chats, list_messages, search_messages. The internal write path
/// (ingest) must NOT appear here — it is an internal scanner path and
/// should never be callable by an agent.
#[test]
fn registered_controllers_exposes_three_agent_tools() {
    let controllers = all_registered_controllers();
    assert_eq!(
        controllers.len(),
        3,
        "expected 3 agent-facing controllers, got {}: {:?}",
        controllers.len(),
        controllers
            .iter()
            .map(|c| c.schema.function)
            .collect::<Vec<_>>()
    );

    let names: Vec<&str> = controllers.iter().map(|c| c.schema.function).collect();
    assert!(
        names.contains(&"list_chats"),
        "list_chats must be registered"
    );
    assert!(
        names.contains(&"list_messages"),
        "list_messages must be registered"
    );
    assert!(
        names.contains(&"search_messages"),
        "search_messages must be registered"
    );
}

/// The ingest handler must NOT be advertised in the agent-facing controller list.
/// Exposing it would allow an agent to mutate or poison the local WhatsApp store.
#[test]
fn ingest_not_in_agent_facing_registry() {
    let controllers = all_registered_controllers();
    let names: Vec<&str> = controllers.iter().map(|c| c.schema.function).collect();
    assert!(
        !names.contains(&"ingest"),
        "ingest must NOT appear in the agent-facing registry, got {names:?}"
    );
}

/// The agent-facing schema list mirrors the controller list.
#[test]
fn controller_schemas_matches_registered_count() {
    let schemas = all_controller_schemas();
    let controllers = all_registered_controllers();
    assert_eq!(
        schemas.len(),
        controllers.len(),
        "schema count ({}) must match registered controller count ({})",
        schemas.len(),
        controllers.len()
    );
}

/// The internal controller set must include ingest (for the Tauri scanner)
/// PLUS the three read-only tools — four in total.
#[test]
fn internal_controllers_includes_ingest_and_read_tools() {
    let internal = all_internal_controllers();
    assert_eq!(
        internal.len(),
        4,
        "expected 4 internal controllers (ingest + 3 read), got {}",
        internal.len()
    );

    let names: Vec<&str> = internal.iter().map(|c| c.schema.function).collect();
    assert!(
        names.contains(&"ingest"),
        "ingest must appear in the internal controller set"
    );
    assert!(names.contains(&"list_chats"));
    assert!(names.contains(&"list_messages"));
    assert!(names.contains(&"search_messages"));
}

/// All registered schemas must use the whatsapp_data namespace.
#[test]
fn all_schemas_use_whatsapp_data_namespace() {
    for schema in all_controller_schemas() {
        assert_eq!(
            schema.namespace, "whatsapp_data",
            "schema '{}' has unexpected namespace '{}'",
            schema.function, schema.namespace
        );
    }
    // Internal set (includes ingest) must also use the same namespace.
    for controller in all_internal_controllers() {
        assert_eq!(
            controller.schema.namespace, "whatsapp_data",
            "internal controller '{}' has unexpected namespace '{}'",
            controller.schema.function, controller.schema.namespace
        );
    }
}

/// search_messages requires a non-optional 'query' field as its first input.
#[test]
fn search_messages_schema_has_required_query_field() {
    use crate::core::TypeSchema;
    let schema = super::schemas("search_messages");
    let query_field = schema
        .inputs
        .iter()
        .find(|f| f.name == "query")
        .expect("search_messages must declare a 'query' input field");
    assert!(query_field.required, "'query' must be required");
    assert!(
        matches!(query_field.ty, TypeSchema::String),
        "'query' must be TypeSchema::String"
    );
}

/// list_messages requires 'chat_id' as a required string field.
#[test]
fn list_messages_schema_has_required_chat_id_field() {
    use crate::core::TypeSchema;
    let schema = super::schemas("list_messages");
    let chat_id = schema
        .inputs
        .iter()
        .find(|f| f.name == "chat_id")
        .expect("list_messages must declare a 'chat_id' input field");
    assert!(chat_id.required, "'chat_id' must be required");
    assert!(
        matches!(chat_id.ty, TypeSchema::String),
        "'chat_id' must be TypeSchema::String"
    );
}

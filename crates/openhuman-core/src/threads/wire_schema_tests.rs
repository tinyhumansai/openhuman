use super::*;

fn resolve_schema<'a>(
    root: &'a serde_json::Value,
    mut schema: &'a serde_json::Value,
) -> &'a serde_json::Value {
    while let Some(reference) = schema.get("$ref").and_then(serde_json::Value::as_str) {
        let pointer = reference.strip_prefix('#').expect("local schema reference");
        schema = root.pointer(pointer).expect("schema reference resolves");
    }
    schema
}

fn schema_string_values(root: &serde_json::Value, schema: &serde_json::Value) -> Vec<String> {
    let schema = resolve_schema(root, schema);
    let mut values: Vec<String> =
        if let Some(values) = schema.get("enum").and_then(serde_json::Value::as_array) {
            values
                .iter()
                .map(|value| value.as_str().expect("string enum value").to_owned())
                .collect()
        } else {
            schema
                .get("oneOf")
                .and_then(serde_json::Value::as_array)
                .expect("string enum or oneOf schema")
                .iter()
                .map(|variant| {
                    variant["const"]
                        .as_str()
                        .expect("string oneOf const")
                        .to_owned()
                })
                .collect()
        };
    values.sort();
    values
}

#[test]
fn thread_wire_schema_exports_the_core_and_session_records() {
    let schema: serde_json::Value =
        serde_json::from_str(&thread_wire_schema_json().expect("schema serializes"))
            .expect("schema is JSON");
    let properties = schema["properties"].as_object().expect("root properties");
    for property in [
        "thread",
        "message",
        "queuedChatAck",
        "startedChatAck",
        "threadsList",
        "threadMessages",
        "threadDelete",
        "purgeResult",
        "threadSearchHit",
        "threadSearch",
        "getTurnState",
        "listTurnStates",
        "clearTurnState",
        "agentRun",
        "runEvent",
        "runTelemetry",
        "turnCitation",
        "queueMode",
        "artifactKind",
        "artifactPendingPayload",
        "artifactReadyPayload",
        "artifactFailedPayload",
        "threadGoal",
        "threadTodoItem",
        "runMode",
    ] {
        assert!(
            properties.contains_key(property),
            "schema is missing {property}"
        );
    }
    assert_eq!(
        schema_string_values(&schema, &properties["queueMode"]),
        vec![
            "collect".to_owned(),
            "followup".to_owned(),
            "interrupt".to_owned(),
            "parallel".to_owned(),
            "steer".to_owned(),
        ]
    );
    assert_eq!(
        schema_string_values(&schema, &properties["artifactKind"]),
        vec![
            "document".to_owned(),
            "image".to_owned(),
            "other".to_owned(),
            "presentation".to_owned(),
            "video".to_owned(),
        ]
    );
    assert_eq!(
        schema_string_values(&schema, &properties["runMode"]),
        vec!["build".to_owned(), "plan".to_owned()]
    );
    let citation_schema = resolve_schema(&schema, &properties["turnCitation"]);
    for field in ["id", "key", "namespace", "score", "timestamp", "snippet"] {
        assert!(
            citation_schema["properties"].get(field).is_some(),
            "citation schema is missing {field}: {citation_schema}"
        );
    }
    for (property, fields) in [
        (
            "artifactPendingPayload",
            &["artifact_id", "kind", "title", "workspace_dir", "path"][..],
        ),
        (
            "artifactReadyPayload",
            &[
                "artifact_id",
                "kind",
                "title",
                "workspace_dir",
                "path",
                "size_bytes",
            ][..],
        ),
        (
            "artifactFailedPayload",
            &["artifact_id", "kind", "title", "workspace_dir", "error"][..],
        ),
    ] {
        let payload = resolve_schema(&schema, &properties[property]);
        for field in fields {
            assert!(
                payload["properties"].get(field).is_some(),
                "{property} schema is missing {field}: {payload}"
            );
        }
        assert_eq!(payload["properties"]["kind"]["type"], "string");
    }
    let ack = resolve_schema(&schema, &properties["queuedChatAck"]);
    for field in [
        "queued",
        "queue_mode",
        "client_id",
        "thread_id",
        "request_id",
        "queue_depth",
        "queued_attachments",
    ] {
        assert!(
            ack["required"].as_array().unwrap().contains(&field.into()),
            "ack schema must require serialized field {field}"
        );
    }
}

#[test]
fn started_chat_ack_schema_requires_every_serialized_field() {
    let schema: serde_json::Value =
        serde_json::from_str(&thread_wire_schema_json().expect("schema serializes"))
            .expect("schema is JSON");
    let properties = schema["properties"].as_object().expect("root properties");
    let ack = resolve_schema(&schema, &properties["startedChatAck"]);
    for field in ["accepted", "client_id", "thread_id", "request_id"] {
        assert!(
            ack["required"].as_array().unwrap().contains(&field.into()),
            "started ack schema must require serialized field {field}"
        );
    }
}

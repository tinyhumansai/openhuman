use super::*;
use serde_json::json;

const ALL_FUNCTIONS: &[&str] = &[
    "list",
    "upsert",
    "create_new",
    "messages_list",
    "message_append",
    "generate_title",
    "message_update",
    "delete",
    "purge",
];

#[test]
fn all_controller_schemas_has_entry_per_function() {
    let names: Vec<_> = all_controller_schemas()
        .into_iter()
        .map(|s| s.function)
        .collect();
    assert_eq!(names.len(), ALL_FUNCTIONS.len());
    for expected in ALL_FUNCTIONS {
        assert!(names.contains(expected), "missing schema for {expected}");
    }
}

#[test]
fn all_registered_controllers_has_handler_per_schema() {
    let controllers = all_registered_controllers();
    assert_eq!(controllers.len(), ALL_FUNCTIONS.len());
    let names: Vec<_> = controllers.iter().map(|c| c.schema.function).collect();
    for expected in ALL_FUNCTIONS {
        assert!(names.contains(expected), "missing handler for {expected}");
    }
}

#[test]
fn every_schema_uses_threads_namespace() {
    for s in all_controller_schemas() {
        assert_eq!(
            s.namespace, "threads",
            "schema {} wrong namespace",
            s.function
        );
    }
}

#[test]
fn unknown_function_returns_fallback() {
    let s = schemas("no_such_fn");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.namespace, "threads");
}

// ── parse::<T>(params) contract ─────────────────────────────────────

fn obj(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(m) => m,
        _ => panic!("expected JSON object"),
    }
}

#[test]
fn parse_upsert_accepts_snake_case_contract() {
    let p: UpsertConversationThreadRequest = parse(obj(json!({
        "id": "t1",
        "title": "Hello",
        "created_at": "2026-04-22T00:00:00Z",
    })))
    .expect("valid snake_case params parse");
    assert_eq!(p.id, "t1");
    assert_eq!(p.title, "Hello");
    assert_eq!(p.created_at, "2026-04-22T00:00:00Z");
}

#[test]
fn parse_upsert_rejects_camel_case_created_at() {
    // Request params contract is snake_case; camelCase must not silently
    // succeed because `createdAt` leaves `created_at` missing and also
    // trips deny_unknown_fields.
    let err = parse::<UpsertConversationThreadRequest>(obj(json!({
        "id": "t1",
        "title": "Hello",
        "createdAt": "2026-04-22T00:00:00Z",
    })))
    .unwrap_err();
    assert!(err.starts_with("invalid params:"), "prefix: {err}");
}

#[test]
fn parse_upsert_rejects_unknown_fields() {
    let err = parse::<UpsertConversationThreadRequest>(obj(json!({
        "id": "t1",
        "title": "Hello",
        "created_at": "2026-04-22T00:00:00Z",
        "extra": "nope",
    })))
    .unwrap_err();
    assert!(err.starts_with("invalid params:"), "prefix: {err}");
    assert!(err.contains("extra"), "field name in error: {err}");
}

#[test]
fn parse_upsert_missing_required_field_errors() {
    let err = parse::<UpsertConversationThreadRequest>(obj(json!({
        "id": "t1",
        "title": "Hello",
    })))
    .unwrap_err();
    assert!(err.starts_with("invalid params:"), "prefix: {err}");
    assert!(err.contains("created_at"), "field name in error: {err}");
}

#[test]
fn parse_messages_list_requires_thread_id() {
    let ok: ConversationMessagesRequest = parse(obj(json!({"thread_id": "t1"}))).unwrap();
    assert_eq!(ok.thread_id, "t1");

    let err = parse::<ConversationMessagesRequest>(obj(json!({}))).unwrap_err();
    assert!(err.contains("thread_id"), "err: {err}");

    // camelCase alias is not accepted under deny_unknown_fields.
    let err = parse::<ConversationMessagesRequest>(obj(json!({"threadId": "t1"}))).unwrap_err();
    assert!(err.starts_with("invalid params:"), "prefix: {err}");
}

#[test]
fn parse_message_append_nested_message_requires_camel_case() {
    // Outer request is snake_case; nested ConversationMessageRecord is
    // camelCase by contract (messageType / createdAt). Assert both paths.
    let ok: AppendConversationMessageRequest = parse(obj(json!({
        "thread_id": "t1",
        "message": {
            "id": "m1",
            "content": "hi",
            "type": "text",
            "sender": "user",
            "createdAt": "2026-04-22T00:00:00Z",
        }
    })))
    .expect("valid nested camelCase message");
    assert_eq!(ok.thread_id, "t1");
    assert_eq!(ok.message.id, "m1");
    assert_eq!(ok.message.created_at, "2026-04-22T00:00:00Z");

    let err = parse::<AppendConversationMessageRequest>(obj(json!({
        "thread_id": "t1",
        "message": {
            "id": "m1",
            "content": "hi",
            "type": "text",
            "sender": "user",
            "created_at": "2026-04-22T00:00:00Z",
        }
    })))
    .unwrap_err();
    assert!(
        err.contains("createdAt"),
        "err surfaces expected key: {err}"
    );
}

#[test]
fn parse_generate_title_assistant_message_is_optional() {
    let without: GenerateConversationThreadTitleRequest =
        parse(obj(json!({"thread_id": "t1"}))).unwrap();
    assert_eq!(without.thread_id, "t1");
    assert_eq!(without.assistant_message, None);

    let with: GenerateConversationThreadTitleRequest = parse(obj(json!({
        "thread_id": "t1",
        "assistant_message": "reply",
    })))
    .unwrap();
    assert_eq!(with.assistant_message.as_deref(), Some("reply"));
}

#[test]
fn parse_message_update_extra_metadata_optional_and_unknown_rejected() {
    let without: UpdateConversationMessageRequest = parse(obj(json!({
        "thread_id": "t1",
        "message_id": "m1",
    })))
    .unwrap();
    assert!(without.extra_metadata.is_none());

    let with: UpdateConversationMessageRequest = parse(obj(json!({
        "thread_id": "t1",
        "message_id": "m1",
        "extra_metadata": {"k": "v"},
    })))
    .unwrap();
    assert_eq!(with.extra_metadata, Some(json!({"k": "v"})));

    let err = parse::<UpdateConversationMessageRequest>(obj(json!({
        "thread_id": "t1",
        "message_id": "m1",
        "bogus": true,
    })))
    .unwrap_err();
    assert!(err.contains("bogus"), "err: {err}");
}

#[test]
fn parse_delete_requires_thread_id_and_deleted_at() {
    let ok: DeleteConversationThreadRequest = parse(obj(json!({
        "thread_id": "t1",
        "deleted_at": "2026-04-22T00:00:00Z",
    })))
    .unwrap();
    assert_eq!(ok.thread_id, "t1");

    let err =
        parse::<DeleteConversationThreadRequest>(obj(json!({"thread_id": "t1"}))).unwrap_err();
    assert!(err.contains("deleted_at"), "err: {err}");
}

#[test]
fn parse_empty_request_rejects_any_field() {
    let _: EmptyRequest = parse(obj(json!({}))).unwrap();
    let err = parse::<EmptyRequest>(obj(json!({"x": 1}))).unwrap_err();
    assert!(err.starts_with("invalid params:"), "prefix: {err}");
}

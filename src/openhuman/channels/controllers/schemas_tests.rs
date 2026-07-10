use super::*;
use serde_json::json;

#[test]
fn schema_handler_parity() {
    let schemas = all_controller_schemas();
    let controllers = all_registered_controllers();
    assert_eq!(
        schemas.len(),
        controllers.len(),
        "schema count must match controller count"
    );

    for (s, c) in schemas.iter().zip(controllers.iter()) {
        assert_eq!(s.namespace, c.schema.namespace);
        assert_eq!(s.function, c.schema.function);
    }
}

#[test]
fn deserialize_connect_params() {
    let params: ConnectParams = serde_json::from_value(json!({
        "channel": "telegram",
        "authMode": "bot_token"
    }))
    .unwrap();
    assert_eq!(params.channel, "telegram");
    assert_eq!(params.auth_mode, "bot_token");
    assert!(params.credentials.is_none());
}

#[test]
fn deserialize_disconnect_params() {
    let params: DisconnectParams = serde_json::from_value(json!({
        "channel": "discord",
        "authMode": "bot_token"
    }))
    .unwrap();
    assert_eq!(params.channel, "discord");
    assert!(!params.clear_memory);
}

#[test]
fn deserialize_disconnect_params_accepts_clear_memory() {
    let params: DisconnectParams = serde_json::from_value(json!({
        "channel": "discord",
        "authMode": "bot_token",
        "clearMemory": true
    }))
    .unwrap();
    assert!(params.clear_memory);
}

#[test]
fn deserialize_status_params_empty() {
    let params: StatusParams = serde_json::from_value(json!({})).unwrap();
    assert!(params.channel.is_none());
}

#[test]
fn deserialize_status_params_with_channel() {
    let params: StatusParams = serde_json::from_value(json!({"channel": "telegram"})).unwrap();
    assert_eq!(params.channel.as_deref(), Some("telegram"));
}

#[test]
fn deserialize_send_message_params() {
    let params: SendMessageParams = serde_json::from_value(json!({
        "channel": "telegram",
        "message": {"text": "hello"}
    }))
    .unwrap();
    assert_eq!(params.channel, "telegram");
}

#[test]
fn to_json_helper() {
    let outcome = RpcOutcome::single_log(json!({"ok": true}), "log");
    assert!(to_json(outcome).is_ok());
}

#[test]
fn connect_logs_preserve_legacy_envelope_cases() {
    assert!(connect_logs("discord", ChannelAuthMode::OAuth, true).is_empty());
    assert_eq!(
        connect_logs("imessage", ChannelAuthMode::ManagedDm, false),
        vec!["stored imessage channel config (local-only)".to_string()]
    );
    assert_eq!(
        connect_logs("telegram", ChannelAuthMode::BotToken, false),
        vec!["stored credentials for channel:telegram:bot_token".to_string()]
    );
}

#[test]
fn raw_or_typed_prefers_raw_payload() {
    let raw = json!({"legacy": true});
    let typed = json!({"typed": true});
    assert_eq!(raw_or_typed(Some(raw.clone()), &typed).unwrap(), raw);
    assert_eq!(raw_or_typed(None, &typed).unwrap(), typed);
}

#[test]
fn schema_adapter_preserves_optional_field_type() {
    let schema = schemas("list_threads");
    let active = schema
        .inputs
        .iter()
        .find(|field| field.name == "active")
        .expect("active input");
    assert!(!active.required);
    assert!(matches!(
        &active.ty,
        TypeSchema::Option(inner) if matches!(inner.as_ref(), TypeSchema::Bool)
    ));
}

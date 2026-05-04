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
fn all_schemas_in_channels_namespace() {
    for schema in all_controller_schemas() {
        assert_eq!(schema.namespace, "channels");
    }
}

#[test]
fn no_duplicate_functions() {
    let schemas = all_controller_schemas();
    let mut fns: Vec<&str> = schemas.iter().map(|s| s.function).collect();
    let len = fns.len();
    fns.sort();
    fns.dedup();
    assert_eq!(fns.len(), len, "duplicate function names found");
}

#[test]
fn every_known_key_resolves_to_non_unknown_schema() {
    let keys = [
        "list",
        "describe",
        "connect",
        "disconnect",
        "status",
        "test",
        "telegram_login_start",
        "telegram_login_check",
        "discord_list_guilds",
        "discord_list_channels",
        "discord_check_permissions",
        "send_message",
        "send_reaction",
        "create_thread",
        "update_thread",
        "list_threads",
    ];
    for k in keys {
        let s = schemas(k);
        assert_eq!(s.namespace, "channels");
        assert_ne!(s.function, "unknown", "key `{k}` fell through");
        assert!(!s.description.is_empty(), "key `{k}` missing description");
        assert!(!s.outputs.is_empty(), "key `{k}` has no outputs");
    }
}

#[test]
fn unknown_function_returns_unknown_fallback() {
    let s = schemas("no_such_fn_123");
    assert_eq!(s.function, "unknown");
    assert_eq!(s.namespace, "channels");
}

#[test]
fn describe_schema_requires_channel() {
    let s = schemas("describe");
    let chan = s.inputs.iter().find(|f| f.name == "channel");
    assert!(chan.is_some_and(|f| f.required));
}

#[test]
fn send_message_requires_channel_and_message() {
    let s = schemas("send_message");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"channel"));
    // The rich-message body is carried in `message` (JSON).
    assert!(required.contains(&"message"));
}

#[test]
fn telegram_login_check_requires_session_id_or_token() {
    let s = schemas("telegram_login_check");
    // Should have at least one required input
    assert!(s.inputs.iter().any(|f| f.required));
}

#[test]
fn discord_list_guilds_schema_may_have_no_required_inputs() {
    let s = schemas("discord_list_guilds");
    // Either no inputs or all-optional inputs are acceptable — but the
    // schema must still exist with outputs.
    assert!(!s.outputs.is_empty());
}

#[test]
fn connect_schema_requires_channel_auth_mode() {
    let s = schemas("connect");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"channel"));
    assert!(required.contains(&"authMode"));
}

#[test]
fn disconnect_schema_requires_channel_auth_mode() {
    let s = schemas("disconnect");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"channel"));
    assert!(required.contains(&"authMode"));
}

#[test]
fn status_schema_has_optional_channel() {
    let s = schemas("status");
    let chan = s.inputs.iter().find(|f| f.name == "channel");
    assert!(chan.is_some_and(|f| !f.required));
}

#[test]
fn test_schema_requires_channel_auth_mode_credentials() {
    let s = schemas("test");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"channel"));
    assert!(required.contains(&"authMode"));
    assert!(required.contains(&"credentials"));
}

#[test]
fn list_schema_has_no_inputs() {
    let s = schemas("list");
    assert!(s.inputs.is_empty());
}

#[test]
fn discord_link_start_schema() {
    let s = schemas("discord_link_start");
    assert_eq!(s.namespace, "channels");
    assert_eq!(s.function, "discord_link_start");
}

#[test]
fn discord_link_check_requires_link_token() {
    let s = schemas("discord_link_check");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"linkToken"));
}

#[test]
fn discord_list_channels_requires_guild_id() {
    let s = schemas("discord_list_channels");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"guildId"));
}

#[test]
fn discord_check_permissions_requires_guild_and_channel() {
    let s = schemas("discord_check_permissions");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"guildId"));
    assert!(required.contains(&"channelId"));
}

#[test]
fn send_reaction_requires_channel_and_reaction() {
    let s = schemas("send_reaction");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"channel"));
    assert!(required.contains(&"reaction"));
}

#[test]
fn create_thread_requires_channel_and_title() {
    let s = schemas("create_thread");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"channel"));
    assert!(required.contains(&"title"));
}

#[test]
fn update_thread_requires_channel_thread_id_action() {
    let s = schemas("update_thread");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"channel"));
    assert!(required.contains(&"threadId"));
    assert!(required.contains(&"action"));
}

#[test]
fn list_threads_requires_channel() {
    let s = schemas("list_threads");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"channel"));
}

#[test]
fn telegram_login_start_schema_has_no_inputs() {
    let s = schemas("telegram_login_start");
    assert!(s.inputs.is_empty());
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
fn required_string_helper() {
    let f = required_string("channel", "channel name");
    assert!(f.required);
    assert!(matches!(f.ty, TypeSchema::String));
}

#[test]
fn optional_string_helper() {
    let f = optional_string("auth_mode", "auth");
    assert!(!f.required);
}

#[test]
fn json_output_helper() {
    let f = json_output("result", "the result");
    assert!(f.required);
    assert!(matches!(f.ty, TypeSchema::Json));
}

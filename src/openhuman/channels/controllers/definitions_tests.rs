use super::*;

#[test]
fn all_definitions_have_unique_ids() {
    let defs = all_channel_definitions();
    let mut ids: Vec<&str> = defs.iter().map(|d| d.id).collect();
    let len = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), len, "duplicate channel definition ids found");
}

#[test]
fn every_definition_has_at_least_one_auth_mode() {
    for def in all_channel_definitions() {
        assert!(
            !def.auth_modes.is_empty(),
            "channel '{}' has no auth modes",
            def.id
        );
    }
}

#[test]
fn required_fields_have_non_empty_key_and_label() {
    for def in all_channel_definitions() {
        for spec in &def.auth_modes {
            for field in &spec.fields {
                if field.required {
                    assert!(
                        !field.key.is_empty(),
                        "empty key in {}.{:?}",
                        def.id,
                        spec.mode
                    );
                    assert!(
                        !field.label.is_empty(),
                        "empty label in {}.{:?}",
                        def.id,
                        spec.mode
                    );
                }
            }
        }
    }
}

#[test]
fn telegram_has_bot_token_and_managed_dm() {
    let def = find_channel_definition("telegram").expect("telegram not found");
    assert!(def.auth_mode_spec(ChannelAuthMode::BotToken).is_some());
    assert!(def.auth_mode_spec(ChannelAuthMode::ManagedDm).is_some());

    let bot = def.auth_mode_spec(ChannelAuthMode::BotToken).unwrap();
    assert!(bot
        .fields
        .iter()
        .any(|f| f.key == "bot_token" && f.required));
    assert!(bot.auth_action.is_none());

    let managed = def.auth_mode_spec(ChannelAuthMode::ManagedDm).unwrap();
    assert_eq!(managed.auth_action, Some("telegram_managed_dm"));
    assert!(managed.fields.is_empty());
}

#[test]
fn discord_has_bot_token_and_oauth() {
    let def = find_channel_definition("discord").expect("discord not found");
    assert!(def.auth_mode_spec(ChannelAuthMode::BotToken).is_some());
    assert!(def.auth_mode_spec(ChannelAuthMode::OAuth).is_some());

    let oauth = def.auth_mode_spec(ChannelAuthMode::OAuth).unwrap();
    assert_eq!(oauth.auth_action, Some("discord_oauth"));

    let managed = def.auth_mode_spec(ChannelAuthMode::ManagedDm);
    assert!(managed.is_some());
    assert_eq!(managed.unwrap().auth_action, Some("discord_managed_link"));
}

#[test]
fn discord_bot_token_exposes_allowed_users_field() {
    // Issue #3763: the Discord bot_token connect UI must offer an allowed_users
    // field (like Telegram) so a self-hosted bot's allowlist is set in-app
    // instead of by hand-editing config.toml.
    let def = find_channel_definition("discord").expect("discord not found");
    let bot_token = def
        .auth_mode_spec(ChannelAuthMode::BotToken)
        .expect("discord bot_token mode");
    let allowed = bot_token
        .fields
        .iter()
        .find(|f| f.key == "allowed_users")
        .expect("discord bot_token must expose an allowed_users field");
    assert!(
        !allowed.required,
        "allowed_users must be optional (blank = allow everyone)"
    );
}

#[test]
fn find_unknown_channel_returns_none() {
    assert!(find_channel_definition("nonexistent").is_none());
}

#[test]
fn validate_credentials_rejects_missing_required() {
    let def = find_channel_definition("telegram").unwrap();
    let empty = serde_json::Map::new();
    let result = def.validate_credentials(ChannelAuthMode::BotToken, &empty);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("bot_token"));
}

#[test]
fn validate_credentials_accepts_complete() {
    let def = find_channel_definition("telegram").unwrap();
    let mut creds = serde_json::Map::new();
    creds.insert(
        "bot_token".to_string(),
        serde_json::Value::String("123:abc".to_string()),
    );
    assert!(def
        .validate_credentials(ChannelAuthMode::BotToken, &creds)
        .is_ok());
}

#[test]
fn validate_credentials_rejects_unsupported_mode() {
    let def = find_channel_definition("telegram").unwrap();
    let empty = serde_json::Map::new();
    let result = def.validate_credentials(ChannelAuthMode::OAuth, &empty);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("does not support"));
}

#[test]
fn serialization_produces_expected_structure() {
    let def = telegram_definition();
    let v = serde_json::to_value(&def).expect("serialize");
    let obj = v.as_object().expect("top-level object");
    assert_eq!(obj.get("id").and_then(|v| v.as_str()), Some("telegram"));
    assert_eq!(
        obj.get("display_name").and_then(|v| v.as_str()),
        Some("Telegram")
    );
    let modes = obj
        .get("auth_modes")
        .and_then(|v| v.as_array())
        .expect("auth_modes");
    assert_eq!(modes.len(), def.auth_modes.len());
    let caps = obj
        .get("capabilities")
        .and_then(|v| v.as_array())
        .expect("capabilities");
    assert_eq!(caps.len(), def.capabilities.len());
}

// -- #2048: Lark / Feishu + DingTalk channel definitions ------------------

#[test]
fn lark_definition_is_registered() {
    let def = find_channel_definition("lark").expect("lark definition not registered");
    assert_eq!(def.display_name, "Lark / Feishu");
    assert_eq!(def.icon, "lark");
}

#[test]
fn lark_uses_api_key_auth_with_app_id_and_secret_required() {
    let def = find_channel_definition("lark").expect("lark not found");
    let spec = def
        .auth_mode_spec(ChannelAuthMode::ApiKey)
        .expect("lark must support ApiKey auth mode");

    // The two non-negotiable fields are app_id + app_secret — every
    // Lark/Feishu open-platform app gets these from the developer console.
    let app_id = spec
        .fields
        .iter()
        .find(|f| f.key == "app_id")
        .expect("missing app_id field");
    assert!(app_id.required, "app_id must be required");
    assert_eq!(app_id.field_type, "string");

    let app_secret = spec
        .fields
        .iter()
        .find(|f| f.key == "app_secret")
        .expect("missing app_secret field");
    assert!(app_secret.required, "app_secret must be required");
    assert_eq!(
        app_secret.field_type, "secret",
        "app_secret must be a secret-typed field"
    );

    // Optional but supported: encrypt_key, verification_token, use_feishu,
    // receive_mode, port, allowed_users. Field names map 1:1 to `LarkConfig`
    // in `src/openhuman/config/schema/channels.rs` — any rename on the
    // backend side will fail this assertion before the UI silently breaks.
    for key in [
        "encrypt_key",
        "verification_token",
        "use_feishu",
        "receive_mode",
        "port",
        "allowed_users",
    ] {
        let field = spec
            .fields
            .iter()
            .find(|f| f.key == key)
            .unwrap_or_else(|| panic!("lark spec missing optional field: {}", key));
        assert!(
            !field.required,
            "lark optional field {} must not be required",
            key
        );
    }

    let use_feishu = spec.fields.iter().find(|f| f.key == "use_feishu").unwrap();
    assert_eq!(use_feishu.field_type, "boolean");
}

#[test]
fn lark_validate_credentials_rejects_missing_app_secret() {
    let def = find_channel_definition("lark").expect("lark not found");
    let mut creds = serde_json::Map::new();
    creds.insert("app_id".into(), serde_json::Value::String("cli_xx".into()));
    // app_secret intentionally omitted.
    let err = def
        .validate_credentials(ChannelAuthMode::ApiKey, &creds)
        .expect_err("must reject when app_secret is missing");
    assert!(err.contains("app_secret"), "{err}");
}

#[test]
fn dingtalk_definition_is_registered() {
    let def = find_channel_definition("dingtalk").expect("dingtalk definition not registered");
    assert_eq!(def.display_name, "DingTalk (钉钉)");
    assert_eq!(def.icon, "dingtalk");
}

#[test]
fn dingtalk_requires_client_id_and_client_secret() {
    let def = find_channel_definition("dingtalk").expect("dingtalk not found");
    let spec = def
        .auth_mode_spec(ChannelAuthMode::ApiKey)
        .expect("dingtalk must support ApiKey auth mode");

    let client_id = spec
        .fields
        .iter()
        .find(|f| f.key == "client_id")
        .expect("missing client_id field");
    assert!(client_id.required);
    assert_eq!(client_id.field_type, "string");

    let client_secret = spec
        .fields
        .iter()
        .find(|f| f.key == "client_secret")
        .expect("missing client_secret field");
    assert!(client_secret.required);
    assert_eq!(
        client_secret.field_type, "secret",
        "client_secret must be a secret-typed field"
    );

    // DingTalkConfig in `src/openhuman/config/schema/channels.rs` also
    // accepts `allowed_users` (Vec<String>, defaults to empty). Pin it
    // as an optional field here for the same reason we pin Lark's
    // optional set — schema renames blow up at test time, not in
    // production UI.
    let allowed_users = spec
        .fields
        .iter()
        .find(|f| f.key == "allowed_users")
        .expect("dingtalk spec missing optional allowed_users field");
    assert!(
        !allowed_users.required,
        "dingtalk allowed_users must not be required"
    );
    assert_eq!(allowed_users.field_type, "string");
}

#[test]
fn dingtalk_validate_credentials_rejects_missing_client_secret() {
    let def = find_channel_definition("dingtalk").expect("dingtalk not found");
    let mut creds = serde_json::Map::new();
    creds.insert(
        "client_id".into(),
        serde_json::Value::String("ding_xx".into()),
    );
    let err = def
        .validate_credentials(ChannelAuthMode::ApiKey, &creds)
        .expect_err("must reject when client_secret is missing");
    assert!(err.contains("client_secret"), "{err}");
}

#[test]
fn all_definitions_include_lark_and_dingtalk() {
    let ids: Vec<&str> = all_channel_definitions().iter().map(|d| d.id).collect();
    assert!(
        ids.contains(&"lark"),
        "lark missing from all_channel_definitions"
    );
    assert!(
        ids.contains(&"dingtalk"),
        "dingtalk missing from all_channel_definitions"
    );
}

#[test]
fn auth_mode_display_and_parse() {
    for mode in [
        ChannelAuthMode::ApiKey,
        ChannelAuthMode::BotToken,
        ChannelAuthMode::OAuth,
        ChannelAuthMode::ManagedDm,
    ] {
        let s = mode.to_string();
        let parsed: ChannelAuthMode = s.parse().expect("parse failed");
        assert_eq!(parsed, mode);
    }
}

#[test]
fn auth_mode_serializes_to_expected_wire_values() {
    assert_eq!(
        serde_json::to_value(ChannelAuthMode::ApiKey).expect("serialize"),
        serde_json::Value::String("api_key".to_string())
    );
    assert_eq!(
        serde_json::from_value::<ChannelAuthMode>(serde_json::Value::String("api_key".to_string()))
            .expect("deserialize"),
        ChannelAuthMode::ApiKey
    );
    assert_eq!(
        serde_json::to_value(ChannelAuthMode::BotToken).expect("serialize"),
        serde_json::Value::String("bot_token".to_string())
    );
    assert_eq!(
        serde_json::from_value::<ChannelAuthMode>(serde_json::Value::String(
            "bot_token".to_string()
        ))
        .expect("deserialize"),
        ChannelAuthMode::BotToken
    );
    assert_eq!(
        serde_json::to_value(ChannelAuthMode::OAuth).expect("serialize"),
        serde_json::Value::String("oauth".to_string())
    );
    assert_eq!(
        serde_json::from_value::<ChannelAuthMode>(serde_json::Value::String("oauth".to_string()))
            .expect("deserialize"),
        ChannelAuthMode::OAuth
    );
    assert_eq!(
        serde_json::to_value(ChannelAuthMode::ManagedDm).expect("serialize"),
        serde_json::Value::String("managed_dm".to_string())
    );
    assert_eq!(
        serde_json::from_value::<ChannelAuthMode>(serde_json::Value::String(
            "managed_dm".to_string()
        ))
        .expect("deserialize"),
        ChannelAuthMode::ManagedDm
    );
}

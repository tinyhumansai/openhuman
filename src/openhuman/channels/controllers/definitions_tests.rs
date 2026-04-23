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

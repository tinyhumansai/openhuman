use super::*;

#[test]
fn origin_strips_path() {
    assert_eq!(
        origin_from_url("https://web.whatsapp.com/").as_deref(),
        Some("https://web.whatsapp.com")
    );
    assert_eq!(
        origin_from_url("https://web.whatsapp.com").as_deref(),
        Some("https://web.whatsapp.com")
    );
    assert_eq!(
        origin_from_url("https://web.whatsapp.com/accounts/42").as_deref(),
        Some("https://web.whatsapp.com")
    );
}

#[test]
fn origin_rejects_malformed() {
    assert!(origin_from_url("web.whatsapp.com").is_none());
    assert!(origin_from_url("://nohost").is_none());
}

#[test]
fn normalize_id_handles_shapes() {
    // Plain string
    assert_eq!(normalize_id(&json!("me@c.us")).as_deref(), Some("me@c.us"));
    // _serialized
    assert_eq!(
        normalize_id(&json!({"_serialized": "a@c.us", "user": "a"})).as_deref(),
        Some("a@c.us")
    );
    // nested id._serialized
    assert_eq!(
        normalize_id(&json!({"id": {"_serialized": "g@g.us"}})).as_deref(),
        Some("g@g.us")
    );
    // id as string
    assert_eq!(
        normalize_id(&json!({"id": "x@c.us"})).as_deref(),
        Some("x@c.us")
    );
    // remote object
    assert_eq!(
        normalize_id(&json!({"remote": {"_serialized": "r@c.us"}})).as_deref(),
        Some("r@c.us")
    );
    // null / missing
    assert!(normalize_id(&json!(null)).is_none());
    assert!(normalize_id(&json!({})).is_none());
    assert!(normalize_id(&json!("")).is_none());
}

#[test]
fn normalize_message_extracts_core_fields() {
    let raw = json!({
        "id": {"_serialized": "false_chat@c.us_MSG1", "fromMe": false},
        "from": "chat@c.us",
        "to": "me@c.us",
        "fromMe": false,
        "t": 1_700_000_000i64,
        "type": "chat",
    });
    let m = normalize_message(&raw).unwrap();
    assert_eq!(m.id, "false_chat@c.us_MSG1");
    assert_eq!(m.chat_id, "chat@c.us");
    assert_eq!(m.from.as_deref(), Some("chat@c.us"));
    assert_eq!(m.to.as_deref(), Some("me@c.us"));
    assert!(!m.from_me);
    assert_eq!(m.timestamp, Some(1_700_000_000));
    assert_eq!(m.type_.as_deref(), Some("chat"));
}

#[test]
fn normalize_message_sets_from_to_me_when_self_sent() {
    let raw = json!({
        "id": "id-1",
        "chatId": "chat@c.us",
        "fromMe": true,
    });
    let m = normalize_message(&raw).unwrap();
    assert_eq!(m.from.as_deref(), Some("me"));
    assert!(m.from_me);
}

#[test]
fn normalize_message_envelope_type_falls_back_to_first_key() {
    let raw = json!({
        "id": "id-2",
        "chatId": "chat@c.us",
        "message": {"imageMessage": {"url": "..."}},
    });
    let m = normalize_message(&raw).unwrap();
    assert_eq!(m.type_.as_deref(), Some("imageMessage"));
}

#[test]
fn normalize_chat_pulls_display_name() {
    let raw = json!({
        "id": "chat@c.us",
        "name": "Chat Display",
    });
    assert_eq!(
        normalize_chat(&raw),
        Some(("chat@c.us".to_string(), "Chat Display".to_string()))
    );
}

#[test]
fn normalize_chat_falls_back_to_contact_pushname() {
    let raw = json!({
        "id": "chat@c.us",
        "contact": {"pushname": "Pushed"},
    });
    assert_eq!(
        normalize_chat(&raw),
        Some(("chat@c.us".to_string(), "Pushed".to_string()))
    );
}

#[test]
fn normalize_contact_prefers_name_then_notify() {
    assert_eq!(
        normalize_contact(&json!({"id": "c@c.us", "name": "Real"})),
        Some(("c@c.us".to_string(), "Real".to_string()))
    );
    assert_eq!(
        normalize_contact(&json!({"id": "c@c.us", "notify": "Notify"})),
        Some(("c@c.us".to_string(), "Notify".to_string()))
    );
}

#[test]
fn requestdata_params_omit_index_name() {
    // Regression guard for Bug 1: passing `indexName: ""` to
    // `IndexedDB.requestData` makes CEF 146 reject the call with
    // "Could not get index". The field must be omitted entirely.
    // Same constraint observed in slack_scanner/idb.rs:210-214 and
    // telegram_scanner/idb.rs:210.
    let params = json!({
        "securityOrigin": "https://web.whatsapp.com",
        "databaseName": "model-storage",
        "objectStoreName": "message",
        "skipCount": 0i64,
        "pageSize": 500i64,
    });
    assert!(
        params.get("indexName").is_none(),
        "indexName must be omitted entirely - passing empty string is rejected by CEF 146 with 'Could not get index' (see slack_scanner/idb.rs:210-214)"
    );
}

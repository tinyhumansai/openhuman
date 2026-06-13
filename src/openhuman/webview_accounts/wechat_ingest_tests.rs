use super::wechat_ingest::{
    list_ingest_envelope, memory_doc_ingest_peer_transcript, validate_scan, WechatChatRow,
    WechatMessageRow, WechatScanPayload,
};

#[test]
fn envelope_includes_provider_and_kind() {
    let payload = WechatScanPayload {
        account_id: "acct-x".into(),
        chat_rows: vec![WechatChatRow {
            name: "Bob".into(),
            preview: Some("ping".into()),
            unread: 1,
        }],
        messages: vec![],
        unread: 1,
        snapshot_key: "deadbeef".into(),
        source: "cdp-dom".into(),
    };
    let env = list_ingest_envelope("acct-x", &payload, 1_234);
    assert_eq!(env["provider"].as_str(), Some("wechat"));
    assert_eq!(env["kind"].as_str(), Some("ingest"));
}

#[test]
fn validate_accepts_messages_only_scan() {
    let payload = WechatScanPayload {
        account_id: "acct".into(),
        chat_rows: vec![],
        messages: vec![WechatMessageRow {
            chat_id: "c1".into(),
            chat_name: "Alice".into(),
            sender: None,
            body: "hello".into(),
            ts: None,
        }],
        unread: 0,
        snapshot_key: String::new(),
        source: "cdp-dom".into(),
    };
    assert!(validate_scan(&payload).is_ok());
}

#[test]
fn peer_transcript_rejects_blank_chat_id() {
    let rows = vec![WechatMessageRow {
        chat_id: "  ".into(),
        chat_name: "x".into(),
        sender: None,
        body: "y".into(),
        ts: None,
    }];
    assert!(memory_doc_ingest_peer_transcript("acct", "  ", "x", &rows).is_err());
}

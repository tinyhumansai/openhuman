use super::*;
use serde_json::json;

// ─── SLACK_FETCH_CONVERSATION_HISTORY ─────────────────────────────────────

#[test]
fn history_reshapes_top_level_messages() {
    let mut data = json!({
        "messages": [
            { "ts": "1714003200.000100", "user": "U1", "text": "hello" },
            { "ts": "1714003300.000200", "user": "U2", "text": "world", "thread_ts": "1714003200.0" },
            { "ts": "1714003400.000300", "user": "U3", "text": "  " }, // dropped: empty text
        ],
        "response_metadata": { "next_cursor": "abc" }
    });
    post_process("SLACK_FETCH_CONVERSATION_HISTORY", None, &mut data);

    let msgs = data["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2, "empty-text message must be dropped");
    assert_eq!(msgs[0]["ts"], "1714003200.000100");
    assert_eq!(msgs[0]["user"], "U1");
    assert_eq!(msgs[0]["text"], "hello");
    assert!(msgs[0].get("thread_ts").is_none());
    assert_eq!(msgs[1]["thread_ts"], "1714003200.0");
}

#[test]
fn history_reshapes_nested_data_envelope() {
    let mut data = json!({
        "data": {
            "messages": [
                { "ts": "1714003200.0", "user": "U1", "text": "hi" }
            ]
        }
    });
    post_process("SLACK_FETCH_CONVERSATION_HISTORY", None, &mut data);
    let msgs = data["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["text"], "hi");
}

#[test]
fn history_reshapes_doubly_nested_envelope() {
    let mut data = json!({
        "data": {
            "data": {
                "messages": [
                    { "ts": "1714003200.0", "user": "U1", "text": "deep" }
                ]
            }
        }
    });
    post_process("SLACK_FETCH_CONVERSATION_HISTORY", None, &mut data);
    let msgs = data["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["text"], "deep");
}

#[test]
fn history_drops_message_without_ts() {
    let mut data = json!({
        "messages": [
            { "user": "U1", "text": "no timestamp" },
            { "ts": "1714003200.0", "user": "U2", "text": "has ts" },
        ]
    });
    post_process("SLACK_FETCH_CONVERSATION_HISTORY", None, &mut data);
    let msgs = data["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["text"], "has ts");
}

// ─── SLACK_LIST_CONVERSATIONS ─────────────────────────────────────────────

#[test]
fn list_conversations_reshapes_channels() {
    let mut data = json!({
        "data": {
            "channels": [
                { "id": "C1", "name": "eng", "is_private": false, "extra": "noise" },
                { "id": "G1", "name": "ops", "is_private": true },
                { "id": "", "name": "empty-id" },  // dropped
            ]
        }
    });
    post_process("SLACK_LIST_CONVERSATIONS", None, &mut data);
    let channels = data["channels"].as_array().unwrap();
    assert_eq!(channels.len(), 2, "empty-id entry must be dropped");
    assert_eq!(channels[0]["id"], "C1");
    assert_eq!(channels[0]["name"], "eng");
    assert_eq!(channels[0]["is_private"], false);
    assert!(
        channels[0].get("extra").is_none(),
        "noise fields must be removed"
    );
    assert_eq!(channels[1]["id"], "G1");
    assert_eq!(channels[1]["is_private"], true);
}

#[test]
fn list_conversations_falls_back_to_conversations_key() {
    let mut data = json!({
        "conversations": [
            { "id": "C2", "name": "dev", "is_private": false }
        ]
    });
    post_process("SLACK_LIST_CONVERSATIONS", None, &mut data);
    let channels = data["channels"].as_array().unwrap();
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0]["id"], "C2");
}

// ─── SLACK_SEARCH_MESSAGES ────────────────────────────────────────────────

#[test]
fn search_messages_reshapes_matches() {
    let mut data = json!({
        "messages": {
            "matches": [
                {
                    "ts": "1714003200.0",
                    "user": "U1",
                    "text": "hello from search",
                    "channel": { "id": "C1" }
                },
                {
                    "ts": "1714003300.0",
                    "user": "U2",
                    "text": "  ",    // dropped: whitespace only
                    "channel": { "id": "C1" }
                },
            ],
            "paging": { "pages": 3 }
        }
    });
    post_process("SLACK_SEARCH_MESSAGES", None, &mut data);
    let msgs = data["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1, "empty-text match must be dropped");
    assert_eq!(msgs[0]["ts"], "1714003200.0");
    assert_eq!(msgs[0]["text"], "hello from search");
    assert_eq!(msgs[0]["channel_id"], "C1");
    assert_eq!(data["pages"], 3, "paging.pages must be preserved");
}

#[test]
fn search_messages_nested_data_envelope() {
    let mut data = json!({
        "data": {
            "messages": {
                "matches": [
                    { "ts": "1714003200.0", "user": "U1", "text": "nested", "channel": { "id": "C2" } }
                ],
                "paging": { "pages": 1 }
            }
        }
    });
    post_process("SLACK_SEARCH_MESSAGES", None, &mut data);
    let msgs = data["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["channel_id"], "C2");
    assert_eq!(data["pages"], 1_u64);
}

#[test]
fn search_messages_no_matches_emits_empty_array() {
    let mut data = json!({ "messages": { "matches": [] } });
    post_process("SLACK_SEARCH_MESSAGES", None, &mut data);
    let msgs = data["messages"].as_array().unwrap();
    assert!(msgs.is_empty());
}

// ─── Unknown slug ─────────────────────────────────────────────────────────

#[test]
fn unknown_slug_is_noop() {
    let mut data = json!({ "foo": "bar" });
    let original = data.clone();
    post_process("SLACK_SEND_MESSAGE", None, &mut data);
    assert_eq!(data, original, "unknown slug must not mutate data");
}

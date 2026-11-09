use super::*;
use serde::{Deserialize, Serialize};
use tinyagents_session::transcript::TranscriptPart;

#[derive(Serialize, Deserialize)]
struct Holder {
    #[serde(with = "history_wire")]
    rows: Vec<TranscriptMessage>,
    #[serde(default, with = "history_wire::option")]
    maybe: Option<Vec<TranscriptMessage>>,
}

#[test]
fn history_wire_writes_only_role_and_content() {
    // A typed tool row is written as the flat envelope string files have
    // always held.
    let mut row = TranscriptMessage::tool_result("c1", "ok");
    row.id = Some("c1".into());
    row.cache_breakpoints = vec![3, 7];
    row.extra_metadata = Some(serde_json::json!({"reasoning_content": "why"}));
    let holder = Holder {
        rows: vec![row],
        maybe: None,
    };
    let written = serde_json::to_value(&holder).unwrap();
    let rows = written["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    let row = rows[0].as_object().unwrap();
    assert_eq!(row.len(), 2);
    assert!(row.contains_key("content"));
    assert!(row.contains_key("role"));
    assert_eq!(rows[0]["role"], "tool");
    // Compared as JSON: key order inside the envelope depends on whether a
    // downstream build unifies serde_json's `preserve_order`.
    let envelope: serde_json::Value =
        serde_json::from_str(rows[0]["content"].as_str().unwrap()).unwrap();
    assert_eq!(
        envelope,
        serde_json::json!({"tool_call_id": "c1", "content": "ok"})
    );
    assert!(written["maybe"].is_null());
}

#[test]
fn history_wire_reads_bare_and_full_rows() {
    let holder: Holder = serde_json::from_value(serde_json::json!({
        "rows": [
            {"role": "user", "content": "hi"},
            {"id": "m1", "role": "system", "content": "s", "cache_breakpoints": [3, 7]},
        ],
        "maybe": [{"role": "assistant", "content": "a"}],
    }))
    .unwrap();
    assert_eq!(holder.rows[0], TranscriptMessage::user("hi"));
    assert_eq!(holder.rows[1].id.as_deref(), Some("m1"));
    assert_eq!(holder.rows[1].cache_breakpoints, vec![3, 7]);
    assert_eq!(holder.maybe, Some(vec![TranscriptMessage::assistant("a")]));
}

#[test]
fn history_wire_option_defaults_to_none_when_absent() {
    let holder: Holder = serde_json::from_value(serde_json::json!({"rows": []})).unwrap();
    assert!(holder.rows.is_empty() && holder.maybe.is_none());
}

#[test]
fn history_wire_lifts_flat_strings_into_typed_rows_and_hands_them_back_exactly() {
    // Key order and `null` content are the writer's, not ours: the string a
    // file held is the string it keeps.
    let envelope =
        "{\"content\":null,\"tool_calls\":[{\"id\":\"c1\",\"name\":\"n\",\"arguments\":\"{}\"}]}";
    let tool = "{\"tool_call_id\":\"c1\",\"content\":\"ok\"}";
    let raw = serde_json::json!({
        "rows": [
            {"role": "assistant", "content": envelope},
            {"role": "tool", "content": tool},
            {"role": "user", "content": "see [IMAGE:data:image/png;base64,AA==] there"},
        ],
        "maybe": null,
    });
    let holder: Holder = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(holder.rows[0].tool_calls.len(), 1);
    assert_eq!(holder.rows[0].content, "");
    assert_eq!(holder.rows[1].tool_call_id.as_deref(), Some("c1"));
    assert_eq!(holder.rows[1].content, "ok");
    // A host-marker image stays text on the wire (the lift to image blocks is
    // the message bridge's).
    assert!(holder.rows[2].parts.is_none());
    assert_eq!(serde_json::to_value(&holder).unwrap(), raw);
}

#[test]
fn history_wire_writes_image_parts_as_host_markers() {
    let row = TranscriptMessage::user_with_parts(vec![
        TranscriptPart::Text {
            text: "see ".into(),
        },
        TranscriptPart::Image {
            url: "data:image/png;base64,AA==".into(),
        },
        TranscriptPart::Text {
            text: " there".into(),
        },
    ]);
    let holder = Holder {
        rows: vec![row],
        maybe: None,
    };
    assert_eq!(
        serde_json::to_value(&holder).unwrap()["rows"][0]["content"],
        "see [IMAGE:data:image/png;base64,AA==] there"
    );
}

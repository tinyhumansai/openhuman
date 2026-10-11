use super::*;
use tinyagents_session::transcript::TranscriptToolCall;
use tinyinference_llm::message::{AssistantMessage, ToolMessage};

#[test]
fn durable_upload_parts_survive_message_row_replay_without_inline_payloads() {
    let mut text = String::from("Inspect these files");
    for (name, mime) in [
        ("image.png", "image/png"),
        ("clip.mp3", "audio/mpeg"),
        ("video.mp4", "video/mp4"),
        ("archive.zip", "application/zip"),
    ] {
        let attachment = crate::agent::attachments::Attachment {
            path: format!("uploads/thread/id/{name}"),
            name: name.into(),
            mime: mime.into(),
            size_bytes: 10,
        };
        text.push_str(&attachment.marker());
    }
    let message = user_message_from_text(&text);
    let row = message_to_native_chat_message(&message).unwrap();
    let replay = chat_message_to_message(&row);
    assert_eq!(message, replay);
    let Message::User(user) = replay else {
        panic!("user row expected")
    };
    assert!(user
        .content
        .iter()
        .any(|part| matches!(part, ContentBlock::Audio(MediaRef::Path { .. }))));
    assert!(user
        .content
        .iter()
        .any(|part| matches!(part, ContentBlock::Video(MediaRef::Path { .. }))));
    assert!(user
        .content
        .iter()
        .any(|part| matches!(part, ContentBlock::Document(MediaRef::Path { .. }))));
    assert!(!serde_json::to_string(&row).unwrap().contains("base64"));
}
use tinyinference_llm::model::ModelRequest;

// #5359: a user turn whose text carries an inline `[IMAGE:data:…]` marker
// (what the multimodal pipeline hands this bridge) must emit a typed
// `ContentBlock::Image` so the provider serializes it as `image_url` — not
// bury the base64 in a `ContentBlock::Text` the model reads as literal text.
#[test]
fn user_image_marker_becomes_an_image_content_block() {
    let png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUg==";
    let msg = TranscriptMessage::user(format!("what is in this screenshot? [IMAGE:{png}]"));

    let Message::User(user) = chat_message_to_message(&msg) else {
        panic!("user role must map to a user message");
    };
    assert_eq!(user.content.len(), 2, "prose text + one image block");
    match &user.content[0] {
        ContentBlock::Text(text) => assert_eq!(text, "what is in this screenshot?"),
        other => panic!("expected the marker-free prose first, got {other:?}"),
    }
    match &user.content[1] {
        ContentBlock::Image(image) => {
            assert_eq!(image.url, png, "the data URI is forwarded verbatim");
            assert_eq!(image.mime_type.as_deref(), Some("image/png"));
        }
        other => panic!("expected an image block, got {other:?}"),
    }
}

#[test]
fn native_image_round_trip_preserves_adjacent_text_for_claude_code() {
    let png = "data:image/png;base64,QUJD";
    let source = Message::User(UserMessage {
        content: vec![
            ContentBlock::Text("before ".to_string()),
            ContentBlock::Image(ImageRef {
                url: png.to_string(),
                mime_type: Some("image/png".to_string()),
            }),
            ContentBlock::Text(" after".to_string()),
        ],
    });
    let stdin = tinyagents_harness::providers::claude_code::render_request_stdin(
        &ModelRequest::new(vec![source]),
        true,
    );
    let line: serde_json::Value = serde_json::from_slice(&stdin).unwrap();
    let content = line["message"]["content"].as_array().unwrap();
    // The current Claude Code bridge preserves adjacent typed blocks without
    // injecting separators.
    assert_eq!(content[0]["text"], "before ");
    assert_eq!(content[1]["type"], "image");
    assert_eq!(content[2]["text"], " after");
}

#[test]
fn native_image_round_trip_preserves_literal_private_marker_text() {
    let source = Message::User(UserMessage {
        content: vec![
            ContentBlock::Text("literal [OH_IMAGE:data:image/png;base64,QUJD]".to_string()),
            ContentBlock::Image(ImageRef {
                url: "data:image/png;base64,REVG".to_string(),
                mime_type: Some("image/png".to_string()),
            }),
        ],
    });
    let stdin = tinyagents_harness::providers::claude_code::render_request_stdin(
        &ModelRequest::new(vec![source]),
        true,
    );
    let line: serde_json::Value = serde_json::from_slice(&stdin).unwrap();
    let content = line["message"]["content"].as_array().unwrap();
    assert_eq!(content[0]["text"], "literal ");
    assert_eq!(content[1]["text"], "[OH_IMAGE:data:image/png;base64,QUJD]");
    assert_eq!(content[2]["type"], "image");
}

// An image-only turn must not emit an empty text block (some providers 400
// on one), and multiple attachments each become their own image block.
#[test]
fn image_only_and_multi_image_user_turns_map_to_image_blocks_only() {
    let jpeg = "data:image/jpeg;base64,/9j/4AAQSkZJRg==";
    let gif = "data:image/gif;base64,R0lGODlhAQABAAAAACw=";

    let Message::User(only) =
        chat_message_to_message(&TranscriptMessage::user(format!("[IMAGE:{jpeg}]")))
    else {
        panic!("user role must map to a user message");
    };
    assert_eq!(only.content.len(), 1);
    assert!(matches!(&only.content[0], ContentBlock::Image(image) if image.url == jpeg));

    // Interleaved prose + images preserve source order: text, image, text,
    // image — so each caption stays next to its image.
    let Message::User(multi) = chat_message_to_message(&TranscriptMessage::user(format!(
        "compare [IMAGE:{jpeg}] and [IMAGE:{gif}]"
    ))) else {
        panic!("user role must map to a user message");
    };
    assert_eq!(multi.content.len(), 4, "text, image, text, image in order");
    assert!(matches!(&multi.content[0], ContentBlock::Text(t) if t == "compare"));
    assert!(matches!(&multi.content[1], ContentBlock::Image(i) if i.url == jpeg));
    assert!(matches!(&multi.content[2], ContentBlock::Text(t) if t == "and"));
    assert!(matches!(&multi.content[3], ContentBlock::Image(i) if i.url == gif));
}

// Legacy paths become typed references; the provider decorator applies policy
// and reads bytes only on its ephemeral request copy.
#[test]
fn local_image_marker_becomes_a_durable_path_reference() {
    let Message::User(user) = chat_message_to_message(&TranscriptMessage::user(
        "see [IMAGE:/tmp/local/path.png] here",
    )) else {
        panic!("user")
    };
    assert_eq!(user.content.len(), 3);
    assert!(
        matches!(&user.content[1], ContentBlock::Image(image) if image.url == "/tmp/local/path.png")
    );
}

// No marker → byte-for-byte the previous behavior: a single text block that
// preserves the original (untrimmed) content.
#[test]
fn plain_user_text_stays_a_single_text_block() {
    let Message::User(user) = chat_message_to_message(&TranscriptMessage::user("  hi there  "))
    else {
        panic!("user role must map to a user message");
    };
    assert_eq!(user.content.len(), 1);
    assert!(matches!(&user.content[0], ContentBlock::Text(text) if text == "  hi there  "));
}

#[test]
fn typed_native_tool_round_maps_to_structured_messages_and_back() {
    // A native tool round is carried in typed fields: calls on the assistant
    // row, the answered call id on the tool row, plain text in `content`.
    let assistant_cm = TranscriptMessage::assistant_with_calls(
        "calling echo",
        vec![TranscriptToolCall {
            id: "call-1".into(),
            name: "echo".into(),
            arguments: r#"{"msg":"hi"}"#.into(),
            extra_content: None,
        }],
    );
    let tool_cm = TranscriptMessage::tool_result("call-1", "echoed:hi");

    let a = chat_message_to_message(&assistant_cm);
    let Message::Assistant(am) = &a else {
        panic!("expected Assistant, got {a:?}");
    };
    assert_eq!(am.tool_calls.len(), 1);
    assert_eq!(am.tool_calls[0].id, "call-1");
    assert_eq!(am.tool_calls[0].name, "echo");
    assert_eq!(
        am.tool_calls[0].arguments,
        serde_json::json!({ "msg": "hi" })
    );
    assert_eq!(a.text(), "calling echo");

    let t = chat_message_to_message(&tool_cm);
    let Message::Tool(tm) = &t else {
        panic!("expected Tool, got {t:?}");
    };
    assert_eq!(tm.tool_call_id, "call-1");
    assert!(!tm.trusted_verbatim);
    assert_eq!(t.text(), "echoed:hi");

    // Back out: the same typed rows, no envelope string anywhere.
    let a_native = message_to_native_chat_message(&a).expect("assistant converts");
    assert_eq!(a_native.role.as_str(), "assistant");
    assert_eq!(a_native.content, "calling echo");
    assert_eq!(a_native.tool_calls, assistant_cm.tool_calls);

    let t_native = message_to_native_chat_message(&t).expect("tool converts");
    assert_eq!(t_native.role.as_str(), "tool");
    assert_eq!(t_native.content, "echoed:hi");
    assert_eq!(t_native.tool_call_id.as_deref(), Some("call-1"));
    assert_eq!(t_native.id.as_deref(), Some("call-1"));
}

/// A legacy envelope row (what an older release wrote) reads into the same
/// model messages as its typed form once the reader has lifted it.
#[test]
fn legacy_envelope_rows_lift_to_the_same_messages_as_typed_rows() {
    let envelope = r#"{"content":"calling echo","tool_calls":[{"id":"call-1","name":"echo","arguments":"{\"msg\":\"hi\"}"}]}"#;
    let tool = r#"{"tool_call_id":"call-1","content":"echoed:hi"}"#;
    let typed = vec![
        TranscriptMessage::assistant_with_calls(
            "calling echo",
            vec![TranscriptToolCall {
                id: "call-1".into(),
                name: "echo".into(),
                arguments: r#"{"msg":"hi"}"#.into(),
                extra_content: None,
            }],
        ),
        TranscriptMessage::tool_result("call-1", "echoed:hi"),
    ];
    let lifted = vec![
        TranscriptMessage::from_legacy("assistant", envelope),
        TranscriptMessage::from_legacy("tool", tool),
    ];
    assert_eq!(history_to_messages(&lifted), history_to_messages(&typed));
    // ...and the lifted rows hand the exact original strings back.
    assert_eq!(lifted[0].legacy_content(), envelope);
    assert_eq!(lifted[1].legacy_content(), tool);
}

#[test]
fn user_text_with_ready_markers_is_stored_as_typed_image_parts() {
    let png = "data:image/png;base64,iVBORw0KGgo=";
    let text = format!("look [IMAGE:{png}] and [IMAGE:/local/path.png] please");
    let msg = user_message_from_text(&text);
    let Message::User(user) = &msg else {
        panic!("user message");
    };
    assert_eq!(user.content.len(), 5);
    assert!(matches!(&user.content[0], ContentBlock::Text(t) if t == "look "));
    assert!(matches!(&user.content[1], ContentBlock::Image(i) if i.url == png));
    assert!(matches!(&user.content[2], ContentBlock::Text(t) if t == " and "));
    assert!(
        matches!(&user.content[3], ContentBlock::Image(image) if image.url == "/local/path.png")
    );
    // The row keeps the parts; `content` is the text.
    let row = message_to_native_chat_message(&msg).expect("row");
    assert_eq!(row.parts.as_ref().map(Vec::len), Some(5));
    assert_eq!(row.content, "look  and  please");
    // The text comes back exactly, markers in place.
    assert_eq!(user_text_with_markers(&msg), text);
    // A row round-trips to the same message.
    assert_eq!(chat_message_to_message(&row), msg);
    // Text without a ready marker is a plain text message.
    for plain in ["hello", "dangling [IMAGE:data:x"] {
        assert_eq!(user_message_from_text(plain), Message::user(plain));
    }
}

#[test]
fn plain_assistant_prose_is_not_misread_as_a_tool_round() {
    let a = chat_message_to_message(&TranscriptMessage::assistant("just a normal reply"));
    let Message::Assistant(am) = &a else {
        panic!("expected Assistant, got {a:?}");
    };
    assert!(am.tool_calls.is_empty());
    assert_eq!(a.text(), "just a normal reply");
}

#[test]
fn reasoning_content_uses_typed_thinking_block_and_round_trips_metadata() {
    let mut chat = TranscriptMessage::assistant("visible answer");
    chat.extra_metadata = Some(serde_json::json!({ REASONING_EXT_KEY: "private thoughts" }));

    let msg = chat_message_to_message(&chat);
    let Message::Assistant(assistant) = &msg else {
        panic!("expected Assistant, got {msg:?}");
    };
    assert_eq!(msg.text(), "visible answer");
    assert!(assistant.content.iter().any(|block| {
        matches!(
            block,
            ContentBlock::Thinking { text, signature: None } if text == "private thoughts"
        )
    }));
    assert!(!assistant
        .content
        .iter()
        .any(|block| matches!(block, ContentBlock::ProviderExtension(_))));

    let back = message_to_chat_message(&msg).expect("assistant converts");
    assert_eq!(back.content, "visible answer");
    assert_eq!(
        back.extra_metadata
            .as_ref()
            .and_then(|meta| meta.get(REASONING_EXT_KEY))
            .and_then(serde_json::Value::as_str),
        Some("private thoughts")
    );
}

#[test]
fn legacy_provider_extension_reasoning_still_round_trips() {
    let msg = Message::Assistant(AssistantMessage {
        id: None,
        content: vec![
            ContentBlock::Text("visible answer".into()),
            ContentBlock::ProviderExtension(
                serde_json::json!({ REASONING_EXT_KEY: "legacy thoughts" }),
            ),
        ],
        tool_calls: vec![],
        usage: None,
        origin: None,
    });

    let back = message_to_chat_message(&msg).expect("assistant converts");
    assert_eq!(back.content, "visible answer");
    assert_eq!(
        back.extra_metadata
            .as_ref()
            .and_then(|meta| meta.get(REASONING_EXT_KEY))
            .and_then(serde_json::Value::as_str),
        Some("legacy thoughts")
    );
}

#[test]
fn roles_round_trip_through_the_bridge() {
    let history = vec![
        TranscriptMessage::system("you are helpful"),
        TranscriptMessage::user("hello"),
        TranscriptMessage::assistant("hi there"),
    ];
    let messages = history_to_messages(&history);
    assert!(matches!(messages[0], Message::System(_)));
    assert!(matches!(messages[1], Message::User(_)));
    assert!(matches!(messages[2], Message::Assistant(_)));

    let back = messages_to_history(&messages);
    assert_eq!(back.len(), 3);
    assert_eq!(back[0].role, "system");
    assert_eq!(back[1].content, "hello");
    assert_eq!(back[2].role, "assistant");
}

#[test]
fn tool_message_preserves_correlation_id() {
    let messages = vec![Message::Tool(ToolMessage {
        tool_call_id: "call-7".into(),
        content: vec![ContentBlock::Text("done".into())],
        trusted_verbatim: false,
        artifact: None,
    })];
    let back = messages_to_history(&messages);
    assert_eq!(back[0].role, "tool");
    assert_eq!(back[0].content, "done");
    assert_eq!(back[0].id.as_deref(), Some("call-7"));
}

#[test]
fn conversation_preserves_tool_call_structure() {
    let messages = vec![
        Message::User(UserMessage {
            content: vec![ContentBlock::Text("do it".into())],
        }),
        Message::Assistant(AssistantMessage {
            id: None,
            content: vec![ContentBlock::Text("calling".into())],
            tool_calls: vec![TaToolCall {
                id: "c1".into(),
                name: "echo".into(),
                arguments: serde_json::json!({"msg": "hi"}),
                invalid: None,
            }],
            usage: None,
            origin: None,
        }),
        Message::Tool(ToolMessage {
            tool_call_id: "c1".into(),
            content: vec![ContentBlock::Text("echoed:hi".into())],
            trusted_verbatim: false,
            artifact: None,
        }),
        Message::Assistant(AssistantMessage {
            id: None,
            content: vec![ContentBlock::Text("all done".into())],
            tool_calls: vec![],
            usage: None,
            origin: None,
        }),
    ];

    // Only the suffix after the last user turn is persisted.
    let suffix = messages_since_last_user(&messages);
    let convo = messages_to_conversation(suffix);
    assert_eq!(convo.len(), 3);
    match &convo[0] {
        TranscriptEntry::AssistantToolCalls { tool_calls, .. } => {
            assert_eq!(tool_calls[0].name, "echo");
            assert_eq!(tool_calls[0].id, "c1");
        }
        other => panic!("expected AssistantToolCalls, got {other:?}"),
    }
    match &convo[1] {
        TranscriptEntry::ToolResults(results) => {
            assert_eq!(results[0].tool_call_id, "c1");
            assert_eq!(results[0].content, "echoed:hi");
        }
        other => panic!("expected ToolResults, got {other:?}"),
    }
    match &convo[2] {
        TranscriptEntry::Chat(c) => {
            assert_eq!(c.role.as_str(), "assistant");
            assert_eq!(c.content, "all done");
        }
        other => panic!("expected Chat, got {other:?}"),
    }
}

#[test]
fn tool_call_convert() {
    let ta = TaToolCall {
        id: "c1".into(),
        name: "echo".into(),
        arguments: serde_json::json!({"msg": "hi"}),
        invalid: None,
    };
    let oh = ta_call_to_oh_call(&ta);
    assert_eq!(oh.id, "c1");
    assert_eq!(oh.name, "echo");
    assert_eq!(oh.arguments, r#"{"msg":"hi"}"#);
}

#[test]
fn reasoning_from_content_keeps_every_thinking_block_in_order() {
    let content = vec![
        ContentBlock::Thinking {
            text: "first span".into(),
            signature: None,
        },
        ContentBlock::Text("visible".into()),
        ContentBlock::Thinking {
            text: "second span".into(),
            signature: None,
        },
        ContentBlock::Thinking {
            text: "   ".into(),
            signature: None,
        },
    ];
    assert_eq!(
        reasoning_from_content(&content).as_deref(),
        Some("first span\n\nsecond span"),
        "every non-empty thinking block is kept, in order"
    );
    assert_eq!(
        reasoning_from_content(&content[..1]).as_deref(),
        Some("first span"),
        "a single block is returned verbatim"
    );
    assert_eq!(reasoning_from_content(&content[1..2]), None);
}

// The flows builder reads a proposal out of `ToolResults` entries. A text
// dialect records a round's results as one `[Tool results]` user row, so the
// history projection must read that replay frame back as `ToolResults` rather
// than leaving it as an opaque user `Chat` (the proposal was lost this way).
#[test]
fn history_projection_reads_text_dialect_replay_frame_as_tool_results() {
    let messages = vec![
        Message::user("build me a flow"),
        Message::user(
            "[Tool results]\n<tool_result id=\"call_1\">\n{\"type\":\"workflow_proposal\"}\n</tool_result>\n",
        ),
        Message::user("plain follow-up that merely mentions [Tool results]"),
    ];

    let projected = messages_to_history_projection(&messages);
    assert_eq!(projected.len(), 3);
    assert!(matches!(&projected[0], TranscriptEntry::Chat(_)));
    match &projected[1] {
        TranscriptEntry::ToolResults(results) => {
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].tool_call_id, "call_1");
            assert_eq!(results[0].content, "{\"type\":\"workflow_proposal\"}");
        }
        other => panic!("expected ToolResults, got {other:?}"),
    }
    assert!(matches!(&projected[2], TranscriptEntry::Chat(_)));
}

// Native tool rounds keep their structure instead of being flattened to chat.
#[test]
fn history_projection_keeps_native_tool_round_structure() {
    let messages = vec![
        Message::user("go"),
        Message::Assistant(AssistantMessage {
            id: None,
            content: vec![],
            tool_calls: vec![TaToolCall {
                id: "c1".into(),
                name: "echo".into(),
                arguments: serde_json::json!({}),
                invalid: None,
            }],
            usage: None,
            origin: None,
        }),
        Message::Tool(ToolMessage {
            tool_call_id: "c1".into(),
            content: vec![ContentBlock::Text("ok".into())],
            trusted_verbatim: false,
            artifact: None,
        }),
    ];
    let projected = messages_to_history_projection(&messages);
    assert!(matches!(
        projected.as_slice(),
        [
            TranscriptEntry::Chat(_),
            TranscriptEntry::AssistantToolCalls { .. },
            TranscriptEntry::ToolResults(_)
        ]
    ));
}

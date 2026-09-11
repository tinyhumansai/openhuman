use super::*;

// #5359: a user turn whose text carries an inline `[IMAGE:data:…]` marker
// (what the multimodal pipeline hands this bridge) must emit a typed
// `ContentBlock::Image` so the provider serializes it as `image_url` — not
// bury the base64 in a `ContentBlock::Text` the model reads as literal text.
#[test]
fn user_image_marker_becomes_an_image_content_block() {
    let png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUg==";
    let msg = ChatMessage::user(format!("what is in this screenshot? [IMAGE:{png}]"));

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

// An image-only turn must not emit an empty text block (some providers 400
// on one), and multiple attachments each become their own image block.
#[test]
fn image_only_and_multi_image_user_turns_map_to_image_blocks_only() {
    let jpeg = "data:image/jpeg;base64,/9j/4AAQSkZJRg==";
    let gif = "data:image/gif;base64,R0lGODlhAQABAAAAACw=";

    let Message::User(only) =
        chat_message_to_message(&ChatMessage::user(format!("[IMAGE:{jpeg}]")))
    else {
        panic!("user role must map to a user message");
    };
    assert_eq!(only.content.len(), 1);
    assert!(matches!(&only.content[0], ContentBlock::Image(image) if image.url == jpeg));

    // Interleaved prose + images preserve source order: text, image, text,
    // image — so each caption stays next to its image.
    let Message::User(multi) = chat_message_to_message(&ChatMessage::user(format!(
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

// A marker whose payload is not a provider-ready reference (a bare path, an
// un-normalized marker) must stay verbatim as text — never sent as an image
// the provider would reject.
#[test]
fn non_data_image_marker_is_kept_as_text() {
    let Message::User(user) = chat_message_to_message(&ChatMessage::user(
        "see [IMAGE:/tmp/local/path.png] here".to_string(),
    )) else {
        panic!("user role must map to a user message");
    };
    assert_eq!(user.content.len(), 1);
    assert!(
        matches!(&user.content[0], ContentBlock::Text(t)
            if t == "see [IMAGE:/tmp/local/path.png] here"),
        "a non-data/http marker stays literal text, got {:?}",
        user.content
    );
}

// No marker → byte-for-byte the previous behavior: a single text block that
// preserves the original (untrimmed) content.
#[test]
fn plain_user_text_stays_a_single_text_block() {
    let Message::User(user) = chat_message_to_message(&ChatMessage::user("  hi there  ")) else {
        panic!("user role must map to a user message");
    };
    assert_eq!(user.content.len(), 1);
    assert!(matches!(&user.content[0], ContentBlock::Text(text) if text == "  hi there  "));
}

#[test]
fn seeded_native_tool_round_recovers_structure_and_round_trips() {
    use crate::openhuman::inference::provider::ToolCall as OhToolCall;
    // The native dispatcher seeds an assistant tool round as a
    // {content, tool_calls} envelope followed by {tool_call_id, content} rows.
    let oh_call = OhToolCall {
        id: "call-1".into(),
        name: "echo".into(),
        arguments: r#"{"msg":"hi"}"#.into(),
        extra_content: None,
    };
    let assistant_cm = ChatMessage::assistant(
        serde_json::json!({ "content": "calling echo", "tool_calls": [oh_call] }).to_string(),
    );
    let tool_cm = ChatMessage::tool(
        serde_json::json!({ "tool_call_id": "call-1", "content": "echoed:hi" }).to_string(),
    );

    // Inbound: the envelopes are recovered into structured harness messages.
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

    // Outbound: re-serialized to a well-formed native tool round (assistant
    // carries structured tool_calls, the tool row carries the matching id).
    let a_native = message_to_native_chat_message(&a);
    assert_eq!(a_native.role, "assistant");
    let av: serde_json::Value = serde_json::from_str(&a_native.content).unwrap();
    assert_eq!(av["tool_calls"][0]["id"], "call-1");
    assert_eq!(av["content"], "calling echo");

    let t_native = message_to_native_chat_message(&t);
    assert_eq!(t_native.role, "tool");
    let tv: serde_json::Value = serde_json::from_str(&t_native.content).unwrap();
    assert_eq!(tv["tool_call_id"], "call-1");
    assert_eq!(tv["content"], "echoed:hi");
}

#[test]
fn plain_assistant_prose_is_not_misread_as_a_tool_round() {
    let a = chat_message_to_message(&ChatMessage::assistant("just a normal reply"));
    let Message::Assistant(am) = &a else {
        panic!("expected Assistant, got {a:?}");
    };
    assert!(am.tool_calls.is_empty());
    assert_eq!(a.text(), "just a normal reply");
}

#[test]
fn reasoning_content_uses_typed_thinking_block_and_round_trips_metadata() {
    let mut chat = ChatMessage::assistant("visible answer");
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

    let back = message_to_chat_message(&msg);
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
    });

    let back = message_to_chat_message(&msg);
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
        ChatMessage::system("you are helpful"),
        ChatMessage::user("hello"),
        ChatMessage::assistant("hi there"),
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
        }),
    ];

    // Only the suffix after the last user turn is persisted.
    let suffix = messages_since_last_user(&messages);
    let convo = messages_to_conversation(suffix);
    assert_eq!(convo.len(), 3);
    match &convo[0] {
        ConversationMessage::AssistantToolCalls { tool_calls, .. } => {
            assert_eq!(tool_calls[0].name, "echo");
            assert_eq!(tool_calls[0].id, "c1");
        }
        other => panic!("expected AssistantToolCalls, got {other:?}"),
    }
    match &convo[1] {
        ConversationMessage::ToolResults(results) => {
            assert_eq!(results[0].tool_call_id, "c1");
            assert_eq!(results[0].content, "echoed:hi");
        }
        other => panic!("expected ToolResults, got {other:?}"),
    }
    match &convo[2] {
        ConversationMessage::Chat(c) => {
            assert_eq!(c.role, "assistant");
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

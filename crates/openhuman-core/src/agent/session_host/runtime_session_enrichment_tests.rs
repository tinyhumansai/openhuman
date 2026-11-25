use super::*;

#[test]
fn host_enrichment_preserves_ordered_typed_media_once() {
    use tinyinference_llm::message::{ContentBlock, ImageRef, MediaRef, Message, UserMessage};

    let original = Message::User(UserMessage {
        content: vec![
            ContentBlock::Text("[Channel context]".into()),
            ContentBlock::Text("before image".into()),
            ContentBlock::Image(ImageRef {
                url: "openhuman-staged://image".into(),
                mime_type: Some("image/png".into()),
            }),
            ContentBlock::Text("between media".into()),
            ContentBlock::Document(MediaRef::Path {
                path: "openhuman-staged://document".into(),
                media_type: Some("text/plain".into()),
            }),
            ContentBlock::Text("after document".into()),
        ],
    });
    let original_text = crate::agent::message_convert::user_text_with_markers(&original);
    let enriched = format!("turn preamble\n\nhost channel context\n{original_text}");
    let Message::User(actual) =
        enrichment::enriched_typed_user_input(&original, &original_text, &enriched)
            .expect("prefix is constructed before original text")
            .expect("typed media is retained")
    else {
        panic!("enriched input remains a user message");
    };

    assert!(
        matches!(&actual.content[0], ContentBlock::Text(text) if text == "turn preamble\n\nhost channel context\n")
    );
    assert!(matches!(&actual.content[1], ContentBlock::Text(text) if text == "[Channel context]"));
    assert!(matches!(&actual.content[2], ContentBlock::Text(text) if text == "before image"));
    assert!(
        matches!(&actual.content[3], ContentBlock::Image(image) if image.url == "openhuman-staged://image")
    );
    assert!(matches!(&actual.content[4], ContentBlock::Text(text) if text == "between media"));
    assert!(
        matches!(&actual.content[5], ContentBlock::Document(MediaRef::Path { path, .. }) if path == "openhuman-staged://document")
    );
    assert!(matches!(&actual.content[6], ContentBlock::Text(text) if text == "after document"));
    assert_eq!(actual.content.len(), 7, "original blocks are appended once");
}

#[test]
fn host_enrichment_fails_closed_if_typed_input_suffix_changes() {
    use tinyinference_llm::message::{ContentBlock, ImageRef, Message, UserMessage};

    let original = Message::User(UserMessage {
        content: vec![ContentBlock::Image(ImageRef {
            url: "openhuman-staged://image".into(),
            mime_type: Some("image/png".into()),
        })],
    });
    assert!(enrichment::enriched_typed_user_input(&original, "original", "changed input").is_err());
}

#[test]
fn host_enrichment_keeps_text_only_path_unchanged() {
    use tinyinference_llm::message::{ContentBlock, Message, UserMessage};

    let original = Message::User(UserMessage {
        content: vec![ContentBlock::Text("hello".into())],
    });
    assert!(
        enrichment::enriched_typed_user_input(&original, "hello", "prefix\n\nhello")
            .expect("text-only input has no typed-media transformation")
            .is_none()
    );
}

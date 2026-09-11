use super::*;

#[test]
fn parses_multiline_chunk() {
    let mut p = StreamJsonParser::new();
    let chunk = r#"{"type":"system","session_id":"s1","schema_version":"2.0"}
{"type":"assistant","message":{"type":"content_block_start","index":0,"content_block":{"type":"text"}}}
"#;
    let events = p.feed(chunk);
    assert_eq!(events.len(), 2);
    assert_eq!(p.schema_version.as_deref(), Some("2.0"));
    assert!(matches!(events[0], ClaudeCodeEvent::System { .. }));
    assert!(matches!(events[1], ClaudeCodeEvent::Assistant { .. }));
}

#[test]
fn handles_split_lines_across_chunks() {
    let mut p = StreamJsonParser::new();
    assert!(p.feed("{\"type\":\"system\"").is_empty());
    assert!(p.feed(",\"session_id\":\"s1\"}").is_empty());
    let events = p.feed("\n");
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], ClaudeCodeEvent::System { .. }));
}

#[test]
fn flushes_trailing_line_on_end() {
    let mut p = StreamJsonParser::new();
    assert!(p
        .feed(r#"{"type":"result","subtype":"success"}"#)
        .is_empty());
    let events = p.end();
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], ClaudeCodeEvent::Result { .. }));
}

#[test]
fn unknown_type_becomes_parse_error() {
    let mut p = StreamJsonParser::new();
    let events = p.feed("{\"type\":\"weird\"}\n");
    assert!(matches!(events[0], ClaudeCodeEvent::ParseError { .. }));
}

#[test]
fn bad_json_becomes_parse_error() {
    let mut p = StreamJsonParser::new();
    let events = p.feed("not json\n");
    assert!(matches!(events[0], ClaudeCodeEvent::ParseError { .. }));
}

/// A stream that ends mid-character must not swallow the partial bytes.
/// Before the carry-over existed, `from_utf8_lossy` turned them into U+FFFD
/// immediately; holding them means `end()` has to release them, or the last
/// line of a truncated stream disappears entirely.
#[test]
fn end_flushes_an_incomplete_trailing_sequence() {
    let line = serde_json::json!({
        "type": "system",
        "session_id": "x",
        "schema_version": "2.0"
    })
    .to_string();
    let mut bytes = line.into_bytes();
    // A lone lead byte: a genuine incomplete tail, and no newline follows.
    bytes.push(0xE4);

    let mut p = StreamJsonParser::new();
    assert!(
        p.feed_bytes(&bytes).is_empty(),
        "no newline yet, nothing emitted"
    );
    let events = p.end();
    assert_eq!(
        events.len(),
        1,
        "the buffered line must still be emitted at EOF"
    );
    // The held byte is released as a replacement character rather than vanishing,
    // which is what the per-chunk decode produced before the carry-over existed.
    //
    // Asserted on the field that carries it, not on the Debug rendering: a
    // `contains` over `{:?}` passes if U+FFFD turns up anywhere in any field, so it
    // would keep passing if the byte were released into the wrong place. The lone
    // 0xE4 lands after the closing brace, which is why this is a `ParseError` and
    // not a `System` event -- the released byte makes the line invalid JSON.
    match &events[0] {
        ClaudeCodeEvent::ParseError { line, .. } => assert!(
            line.ends_with(char::REPLACEMENT_CHARACTER),
            "the released byte should be the last character of the line, got {line:?}"
        ),
        other => panic!("expected the unparsable line to be reported, got {other:?}"),
    }
}

/// An invalid byte earlier in the chunk must not cost the incomplete
/// character at the end of it.
///
/// Handing everything after the first bad byte to `from_utf8_lossy` replaces
/// a trailing partial sequence before its other half arrives — the exact
/// corruption `pending` exists to prevent, reachable again through one stray
/// byte anywhere earlier in the same read.
#[test]
fn an_invalid_byte_does_not_consume_a_split_character_after_it() {
    let mut parser = StreamJsonParser::new();

    // An ASCII marker holds the spot while the fixture is built as valid
    // JSON, and is overwritten with a real invalid byte afterwards. (A NUL
    // would not survive: serde escapes it to `\u0000`, six ASCII bytes.)
    let line = serde_json::json!({
        "type": "assistant",
        "message": { "type": "text", "text": "bad@then 🌍 tail" }
    })
    .to_string()
        + "\n";

    let mut bytes: Vec<u8> = line.into_bytes();
    let bad = bytes
        .windows(8)
        .position(|w| w == b"bad@then")
        .expect("fixture contains the marker")
        + 3;
    // A lone lead byte of a 3-byte sequence: never valid on its own.
    bytes[bad] = 0xE4;

    let emoji_start = bytes
        .windows(4)
        .position(|w| w == "🌍".as_bytes())
        .expect("fixture contains the emoji");
    // Split inside the emoji, which sits after the invalid byte.
    let split = emoji_start + 2;
    assert!(
        split > bad,
        "precondition: the invalid byte has to come first for this to test anything"
    );

    let first = parser.feed_bytes(&bytes[..split]);
    assert!(
        first.is_empty(),
        "no newline in the first chunk, so nothing should be emitted yet"
    );

    let events = parser.feed_bytes(&bytes[split..]);
    assert_eq!(events.len(), 1, "the completed line must be emitted");

    match &events[0] {
        ClaudeCodeEvent::Assistant { message } => {
            let text = message
                .get("text")
                .and_then(Value::as_str)
                .expect("the fixture carries the text field");
            assert!(
                text.contains('🌍'),
                "the split emoji must survive an invalid byte earlier in the chunk, got {text:?}"
            );
            assert!(
                text.contains(char::REPLACEMENT_CHARACTER),
                "the invalid byte should still surface as a replacement, got {text:?}"
            );
        }
        other => panic!("expected the assistant line to parse, got {other:?}"),
    }
}

/// `feed_bytes` is handed arbitrary read boundaries (the driver reads into an
/// 8 KiB buffer), so a multi-byte character can straddle two chunks. Decoding
/// each chunk independently replaced both halves with U+FFFD, and because that
/// character is legal inside a JSON string the line still parsed -- the text was
/// corrupted with no error raised anywhere.
#[test]
fn feed_bytes_preserves_a_character_split_across_chunks() {
    let line = serde_json::json!({
        "type": "assistant",
        "message": { "type": "text", "text": "héllo 世界 🌍 tail" }
    })
    .to_string()
        + "\n";

    // Split inside the emoji: its four bytes land on both sides of the boundary.
    let bytes = line.as_bytes();
    let emoji_start = line.find('🌍').expect("fixture contains the emoji");
    let split = emoji_start + 2;
    assert!(
        !line.is_char_boundary(split),
        "the split must be mid-character"
    );

    let mut p = StreamJsonParser::new();
    let mut events = p.feed_bytes(&bytes[..split]);
    events.extend(p.feed_bytes(&bytes[split..]));

    assert_eq!(events.len(), 1);

    // `ParseError` carries the original line, so its Debug rendering contains
    // the emoji too — matching the variant is what separates "decoded across
    // the boundary" from "failed to parse and echoed the bytes back".
    let ClaudeCodeEvent::Assistant { message } = &events[0] else {
        panic!("expected an Assistant event, got {:?}", events[0]);
    };
    assert_eq!(
        message["text"], "héllo 世界 🌍 tail",
        "the character split across the two chunks must survive intact"
    );

    let rendered = format!("{:?}", events[0]);
    assert!(
        rendered.contains("héllo 世界 🌍 tail"),
        "text was corrupted across the chunk boundary: {rendered}"
    );
    assert!(
        !rendered.contains('\u{FFFD}'),
        "replacement character present: {rendered}"
    );
}

/// Bytes that can never become valid must not stall the stream: the old lossy
/// behaviour is kept for those, so only an *incomplete tail* is carried over.
#[test]
fn feed_bytes_still_replaces_genuinely_invalid_bytes() {
    let mut p = StreamJsonParser::new();
    let mut chunk: Vec<u8> = b"{\"type\":\"system\",\"session_id\":\"".to_vec();
    chunk.push(0xFF); // not a valid UTF-8 lead byte in any position
    chunk.extend_from_slice(b"\",\"schema_version\":\"2.0\"}\n");
    let events = p.feed_bytes(&chunk);
    assert_eq!(
        events.len(),
        1,
        "the line must still be emitted, not withheld"
    );

    // Not just "an event arrived": the invalid byte has to have been REPLACED.
    // Dropping it would also emit one event and would also parse, so the count
    // alone cannot tell the two apart.
    let ClaudeCodeEvent::System { session_id, .. } = &events[0] else {
        panic!("expected a System event, got {:?}", events[0]);
    };
    assert_eq!(
        session_id.as_deref(),
        Some("\u{FFFD}"),
        "0xFF must decode to the replacement character, not vanish"
    );
}

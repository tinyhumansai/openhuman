use super::*;
use serde_json::json;

fn fixture_with_backend_markdown() -> Value {
    json!({
        "messages": [
            {
                "messageId": "m1",
                "threadId": "t1",
                "subject": "Hello",
                "sender": "a@x.com",
                "to": "b@y.com",
                "messageTimestamp": "2026-04-17T12:00:00Z",
                "labelIds": ["INBOX", "UNREAD"],
                // Pre-rendered slice (set by `apply_response_level_markdown`
                // in production; inline here for the reshape test).
                "markdownFormatted": "# Hello\n\nbody copy",
                "messageText": "fallback should not be used",
                "display_url": "ignore-me",
                "preview": { "body": "Hi plain", "subject": "Hello" },
                "attachmentList": [
                    { "filename": "report.pdf", "mimeType": "application/pdf", "size": 12345 },
                    { "filename": "", "mimeType": "text/html" }
                ],
                "payload": {}
            }
        ],
        "nextPageToken": "tok-1",
        "resultSizeEstimate": 42
    })
}

#[test]
fn reshape_emits_slim_envelope() {
    let mut v = fixture_with_backend_markdown();
    post_process("GMAIL_FETCH_EMAILS", None, &mut v);

    assert_eq!(v["nextPageToken"], "tok-1");
    assert_eq!(v["resultSizeEstimate"], 42);

    let msgs = v["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    let m = &msgs[0];

    assert_eq!(m["id"], "m1");
    assert_eq!(m["threadId"], "t1");
    assert_eq!(m["subject"], "Hello");
    assert_eq!(m["from"], "a@x.com");
    assert_eq!(m["to"], "b@y.com");
    assert_eq!(m["date"], "2026-04-17T12:00:00Z");
    assert_eq!(m["labels"], json!(["INBOX", "UNREAD"]));

    let md = m["markdown"].as_str().unwrap();
    assert_eq!(md, "# Hello\n\nbody copy");

    // Noise fields removed.
    assert!(m.get("display_url").is_none());
    assert!(m.get("preview").is_none());
    assert!(m.get("payload").is_none());
    assert!(m.get("messageText").is_none());

    // Attachments: empty filename entry is filtered.
    let atts = m["attachments"].as_array().unwrap();
    assert_eq!(atts.len(), 1);
    assert_eq!(atts[0]["filename"], "report.pdf");
    assert_eq!(atts[0]["mimeType"], "application/pdf");
}

#[test]
fn raw_html_flag_passes_through_unchanged() {
    let mut v = fixture_with_backend_markdown();
    let original = v.clone();
    let args = json!({ "raw_html": true });
    post_process("GMAIL_FETCH_EMAILS", Some(&args), &mut v);
    assert_eq!(
        v, original,
        "raw_html=true must preserve the Composio shape"
    );
}

#[test]
fn camel_case_raw_html_also_recognized() {
    let mut v = fixture_with_backend_markdown();
    let original = v.clone();
    let args = json!({ "rawHtml": true });
    post_process("GMAIL_FETCH_EMAILS", Some(&args), &mut v);
    assert_eq!(v, original);
}

#[test]
fn falls_back_to_message_text_when_no_backend_markdown() {
    let mut v = json!({
        "messages": [{
            "messageId": "m1",
            "threadId": "t1",
            "subject": "s",
            "sender": "a@x.com",
            "to": "b@y.com",
            "messageTimestamp": "2026-04-17",
            "labelIds": [],
            "messageText": "  plain body text  ",
            "payload": {}
        }],
        "nextPageToken": null
    });
    post_process("GMAIL_FETCH_EMAILS", None, &mut v);
    let md = v["messages"][0]["markdown"].as_str().unwrap();
    assert_eq!(md, "plain body text");
    assert!(v.get("nextPageToken").is_none(), "null tokens dropped");
}

#[test]
fn unwraps_data_envelope() {
    let mut v = json!({
        "data": {
            "messages": [{
                "messageId": "m1",
                "threadId": "t1",
                "subject": "s",
                "sender": "a@x.com",
                "to": "b@y.com",
                "messageTimestamp": "2026-04-17",
                "labelIds": [],
                "messageText": "body",
                "payload": {}
            }]
        }
    });
    post_process("GMAIL_FETCH_EMAILS", None, &mut v);
    // Reshape writes into `data` in place.
    let msgs = v["data"]["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["markdown"], "body");
}

#[test]
fn non_fetch_slug_is_noop() {
    let mut v = json!({ "messages": [{ "messageId": "m1", "messageText": "x" }] });
    let original = v.clone();
    post_process("GMAIL_SEND_EMAIL", None, &mut v);
    assert_eq!(v, original);
}

#[test]
fn prefers_backend_markdown_formatted_when_present() {
    // Composio backend (tinyhumansai/backend#683 +) ships
    // `markdownFormatted` already URL-shortened + footer-stripped
    // per message (after `apply_response_level_markdown` slices the
    // response-level field). When present, our post-processor must
    // use it verbatim instead of falling back to `messageText`.
    let mut v = json!({
        "messages": [{
            "messageId": "m1",
            "threadId": "t1",
            "subject": "s",
            "sender": "a@x.com",
            "to": "b@y.com",
            "messageTimestamp": "2026-04-17",
            "labelIds": [],
            "markdownFormatted": "# Already nice\n\nShort URL: https://gh.io/abc",
            "messageText": "fallback should not be used",
            "payload": {}
        }]
    });
    post_process("GMAIL_FETCH_EMAILS", None, &mut v);
    let md = v["messages"][0]["markdown"].as_str().unwrap();
    assert_eq!(md, "# Already nice\n\nShort URL: https://gh.io/abc");
}

#[test]
fn empty_markdown_formatted_falls_through_to_message_text() {
    let mut v = json!({
        "messages": [{
            "messageId": "m1",
            "threadId": "t1",
            "subject": "s",
            "sender": "a@x.com",
            "to": "b@y.com",
            "messageTimestamp": "2026-04-17",
            "labelIds": [],
            "markdownFormatted": "   \n  \n",
            "messageText": "real body",
            "payload": {}
        }]
    });
    post_process("GMAIL_FETCH_EMAILS", None, &mut v);
    let md = v["messages"][0]["markdown"].as_str().unwrap();
    assert!(md.contains("real body"));
}

// ── split_response_markdown_per_message ─────────────────────────────────

#[test]
fn split_response_markdown_uses_horizontal_rule_marker() {
    // The confirmed backend marker is `\n---\n`. Three messages →
    // expect three slices when there's no preamble.
    let md = "## Alice's update\n\nbody A with https://gh.io/abc\n---\n## Bob's reply\n\nbody B\n---\n## Carol\n\nbody C";
    let slices = super::split_response_markdown_per_message(md, 3).unwrap();
    assert_eq!(slices.len(), 3);
    assert!(slices[0].contains("Alice's update"));
    assert!(slices[1].contains("Bob's reply"));
    assert!(slices[2].contains("Carol"));
    // The `---\n` prefix is preserved on every-but-the-first segment
    // so the section break survives the round-trip.
    assert!(slices[1].starts_with("---\n"));
    assert!(slices[2].starts_with("---\n"));
}

#[test]
fn split_response_markdown_drops_preamble() {
    // When a preamble like `# Inbox` precedes the first marker, we
    // see N+1 parts after split — the preamble must be dropped.
    let md = "# Inbox (2 messages)\n---\n## A\n\nbody A\n---\n## B\n\nbody B";
    let slices = super::split_response_markdown_per_message(md, 2).unwrap();
    assert_eq!(slices.len(), 2);
    assert!(slices[0].contains("body A"));
    assert!(slices[1].contains("body B"));
    // Both segments should carry the prefix when preamble was dropped.
    assert!(slices[0].starts_with("---\n"));
    assert!(slices[1].starts_with("---\n"));
}

#[test]
fn split_response_markdown_falls_back_to_h2_marker() {
    // No `---` rules — backend used h2 headings as boundaries.
    let md = "## Alice\n\nbody A\n\n## Bob\n\nbody B";
    let slices = super::split_response_markdown_per_message(md, 2).unwrap();
    assert_eq!(slices.len(), 2);
    assert!(slices[0].contains("body A"));
    assert!(slices[1].contains("body B"));
}

#[test]
fn split_response_markdown_returns_none_on_count_mismatch() {
    let md = "## only one section here";
    assert!(super::split_response_markdown_per_message(md, 3).is_none());
}

#[test]
fn split_response_markdown_single_message_returns_whole_input() {
    let md = "## solo\n\nthe whole body";
    let slices = super::split_response_markdown_per_message(md, 1).unwrap();
    assert_eq!(slices, vec![md.to_string()]);
}

#[test]
fn split_with_hint_rejects_when_subjects_dont_match() {
    let md = "## Foo\nbody1\n---\n## Bar\nbody2";
    let hints = vec![
        json!({"subject": "Completely different subject A"}),
        json!({"subject": "Completely different subject B"}),
    ];
    let out = super::split_response_markdown_per_message_with_hint(md, 2, Some(&hints));
    assert!(out.is_none(), "subject mismatch must force fallback");
}

#[test]
fn split_with_hint_accepts_when_subjects_match() {
    let md = "## Welcome to Gmail\nbody1\n---\n## Your invoice\nbody2";
    let hints = vec![
        json!({"subject": "Welcome to Gmail"}),
        json!({"subject": "Your invoice"}),
    ];
    let slices = super::split_response_markdown_per_message_with_hint(md, 2, Some(&hints)).unwrap();
    assert_eq!(slices.len(), 2);
    assert!(slices[0].contains("Welcome to Gmail"));
    assert!(slices[1].contains("Your invoice"));
}

#[test]
fn split_with_hint_skips_messages_with_blank_subject() {
    let md = "## A\nbody1\n---\n## B\nbody2";
    let hints = vec![json!({"subject": "A"}), json!({"subject": ""})];
    let slices = super::split_response_markdown_per_message_with_hint(md, 2, Some(&hints)).unwrap();
    assert_eq!(slices.len(), 2);
}

#[test]
fn apply_response_level_markdown_stashes_per_message_field() {
    let mut data = json!({
        "messages": [
            {"messageId": "m1", "subject": "Hello"},
            {"messageId": "m2", "subject": "World"},
        ]
    });
    let top_md = "## Hello\nbody A — link https://gh.io/abc\n---\n## World\nbody B";
    super::apply_response_level_markdown(&mut data, top_md);
    let m1 = data["messages"][0]["markdownFormatted"].as_str().unwrap();
    let m2 = data["messages"][1]["markdownFormatted"].as_str().unwrap();
    assert!(m1.contains("Hello"));
    assert!(
        m1.contains("https://gh.io/abc"),
        "shortened URL must survive"
    );
    assert!(m2.contains("World"));
    assert!(!m1.contains("World"), "no cross-message bleed");
}

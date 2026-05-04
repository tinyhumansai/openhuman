use super::*;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::json;

fn b64(s: &str) -> String {
    URL_SAFE_NO_PAD.encode(s.as_bytes())
}

fn fixture() -> Value {
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
                "messageText": "Hi plain",
                "display_url": "ignore-me",
                "preview": { "body": "Hi plain", "subject": "Hello" },
                "attachmentList": [
                    { "filename": "report.pdf", "mimeType": "application/pdf", "size": 12345 },
                    { "filename": "", "mimeType": "text/html" }
                ],
                "payload": {
                    "headers": [ { "name": "Date", "value": "Fri, 17 Apr 2026 12:00:00 +0000" } ],
                    "parts": [
                        {
                            "mimeType": "text/plain",
                            "body": { "data": b64("Hi plain text") }
                        },
                        {
                            "mimeType": "text/html",
                            "body": { "data": b64("<h1>Title</h1><p>Hello <b>world</b></p>") }
                        }
                    ]
                }
            }
        ],
        "nextPageToken": "tok-1",
        "resultSizeEstimate": 42
    })
}

#[test]
fn reshape_emits_slim_envelope() {
    let mut v = fixture();
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
    assert!(
        md.contains("Title"),
        "markdown body must carry heading text: {md:?}"
    );
    assert!(md.contains("Hello"));
    assert!(md.contains("world"));
    assert!(!md.contains("<h1>"), "html must be stripped: {md:?}");

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
    let mut v = fixture();
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
    let mut v = fixture();
    let original = v.clone();
    let args = json!({ "rawHtml": true });
    post_process("GMAIL_FETCH_EMAILS", Some(&args), &mut v);
    assert_eq!(v, original);
}

#[test]
fn falls_back_to_message_text_when_no_parts() {
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
fn nested_multipart_html_is_found() {
    let html = "<p>Deep <b>body</b></p>";
    let mut v = json!({
        "messages": [{
            "messageId": "m1",
            "threadId": "t1",
            "subject": "s",
            "sender": "a@x.com",
            "to": "b@y.com",
            "messageTimestamp": "2026-04-17",
            "labelIds": [],
            "messageText": "",
            "payload": {
                "parts": [
                    {
                        "mimeType": "multipart/alternative",
                        "parts": [
                            { "mimeType": "text/plain", "body": { "data": b64("plain fallback") } },
                            { "mimeType": "text/html",  "body": { "data": b64(html) } }
                        ]
                    }
                ]
            }
        }]
    });
    post_process("GMAIL_FETCH_EMAILS", None, &mut v);
    let md = v["messages"][0]["markdown"].as_str().unwrap();
    assert!(md.contains("Deep"));
    assert!(md.contains("body"));
    assert!(!md.contains("<p>"));
}

#[test]
fn strip_excess_blank_lines_collapses_runs() {
    let input = "a\n\n\n\nb\n\n\nc\n";
    assert_eq!(strip_excess_blank_lines(input), "a\n\nb\n\nc");
}

#[test]
fn large_html_uses_fast_strip_fallback() {
    let html = format!(
        "<html><head><style>.x{{color:red}}</style></head><body>{}</body></html>",
        (0..3000)
            .map(|i| format!("<div><h2>Card {i}</h2><p>Hello &amp; welcome&nbsp;home</p></div>"))
            .collect::<String>()
    );
    let md = html_email_to_markdown(&html);
    assert!(
        md.contains("## Card 0"),
        "heading should survive fallback: {md:?}"
    );
    assert!(md.contains("Hello & welcome home"));
    assert!(!md.contains("<div>"), "html tags must be stripped: {md:?}");
    assert!(
        !md.contains(".x{color:red}"),
        "style blocks must be removed: {md:?}"
    );
}

#[test]
fn normalize_markdownish_text_removes_invisible_and_extra_spaces() {
    let input = " Hello\u{200b}   world \n\n  line\u{00a0}two ";
    assert_eq!(normalize_markdownish_text(input), "Hello world\n\nline two");
}

#[test]
fn sanitize_llm_text_strips_weird_token_wasting_chars() {
    let input = "A\u{200b}\u{200d}\u{2060}\u{feff}\u{00ad}B\u{202e}C\tD\nE";
    assert_eq!(sanitize_llm_text(input), "ABC\tD\nE");
}

#[test]
fn decode_entities_inline_handles_named_and_numeric() {
    let s = "Terms &amp; Conditions &nbsp; and &#169; 2026 with &#x2014; dash";
    let decoded = decode_html_entities_inline(s);
    assert!(decoded.contains("Terms & Conditions"));
    assert!(decoded.contains("© 2026"));
    assert!(decoded.contains("— dash"));
    assert!(!decoded.contains("&amp;"));
    assert!(!decoded.contains("&#169;"));
}

#[test]
fn decode_entities_inline_preserves_unknown() {
    let s = "keep &notarealentity; and & without semi";
    assert_eq!(decode_html_entities_inline(s), s);
}

#[test]
fn unescape_markdown_backslashes_strips_known_escapes() {
    let s = r"a\&b \_ c \. d \\ e \n";
    let out = unescape_markdown_backslashes(s);
    // Known escapes drop the backslash; unknown (`\\` before letter n) stays.
    assert!(out.contains("a&b"));
    assert!(out.contains("_"));
    assert!(out.contains(". d"));
    assert!(out.contains(r"\\ e"));
    assert!(out.contains(r"\n"));
}

#[test]
fn collapse_separator_runs_squashes_noise() {
    assert_eq!(collapse_separator_runs("x & & & & y"), "x & y");
    assert_eq!(collapse_separator_runs("- - - header"), "- header");
    assert_eq!(collapse_separator_runs("a | | | b"), "a | b");
    // Preserves legitimate single-use separators.
    assert_eq!(
        collapse_separator_runs("Terms & Conditions"),
        "Terms & Conditions"
    );
    // Multi-char tokens are untouched.
    assert_eq!(collapse_separator_runs("a -- b"), "a -- b");
}

#[test]
fn normalize_cleans_entity_and_separator_noise() {
    let input = "Terms &amp; &amp; &amp; &amp; Conditions \
        with &nbsp; &nbsp; spaces and\\& an escaped ampersand";
    let out = normalize_markdownish_text(input);
    assert!(out.contains("Terms & Conditions"), "got: {out:?}");
    assert!(!out.contains("&amp;"));
    assert!(!out.contains("&nbsp;"));
    assert!(out.contains("an escaped ampersand"));
}

#[test]
fn detects_raw_html_like_output() {
    assert!(looks_like_raw_html("<html><body>hello</body></html>"));
    assert!(!looks_like_raw_html("# Heading\n\nBody text"));
}

#[test]
fn html_in_message_text_is_converted() {
    let mut v = json!({
        "messages": [{
            "messageId": "m1",
            "threadId": "t1",
            "subject": "s",
            "sender": "a@x.com",
            "to": "b@y.com",
            "messageTimestamp": "2026-04-17",
            "labelIds": [],
            "messageText": "<html><body><h1>Hello</h1><p>World</p></body></html>",
            "payload": {}
        }]
    });
    post_process("GMAIL_FETCH_EMAILS", None, &mut v);
    let md = v["messages"][0]["markdown"].as_str().unwrap();
    assert!(md.contains("Hello"));
    assert!(md.contains("World"));
    assert!(!md.contains("<html>"));
}

#[test]
fn suspiciously_short_markdown_detects_large_collapse() {
    assert!(suspiciously_short_markdown(&"x".repeat(4000), "tiny"));
    assert!(!suspiciously_short_markdown(
        &"x".repeat(4000),
        &"y".repeat(400)
    ));
}

#[test]
fn fast_html_strip_handles_long_tags() {
    let long_href = format!(
        "<a href=\"https://example.com/{}\">Click me</a><p>After link</p>",
        "x".repeat(600)
    );
    let md = fast_html_email_to_markdown(&long_href);
    assert!(md.contains("Click me"));
    assert!(md.contains("After link"));
}

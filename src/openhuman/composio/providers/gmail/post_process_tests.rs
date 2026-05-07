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
                        // Realistic MIME multipart/alternative: text/plain
                        // mirrors text/html content. Author-provided
                        // plaintext is what we now prefer for downstream
                        // ingestion (see extract_markdown_body docstring).
                        {
                            "mimeType": "text/plain",
                            "body": { "data": b64("Title\n\nHello world") }
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
fn nested_multipart_prefers_plaintext_over_html() {
    // When a multipart/alternative ships BOTH text/plain and text/html, the
    // plaintext part wins. text/plain is the author's intended fallback,
    // bypasses HTML stripping on the sibling `text/html` part (see
    // post_process.rs::extract_markdown_body), and is generally
    // cleaner input for downstream LLM extraction.
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
    assert!(
        md.contains("plain fallback"),
        "plaintext should win, got: {md:?}"
    );
    // The HTML body should NOT appear — the text/html branch was bypassed.
    assert!(
        !md.contains("Deep"),
        "html should not have been used: {md:?}"
    );
    assert!(
        !md.contains("<p>"),
        "raw html should never leak through: {md:?}"
    );
}

#[test]
fn nested_multipart_falls_back_to_html_when_no_plaintext() {
    // For html-only emails (rare — some poorly-built marketing senders),
    // we run the bounded linear HTML strip (see `html_email_to_markdown`).
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
                            { "mimeType": "text/html", "body": { "data": b64(html) } }
                        ]
                    }
                ]
            }
        }]
    });
    post_process("GMAIL_FETCH_EMAILS", None, &mut v);
    let md = v["messages"][0]["markdown"].as_str().unwrap();
    assert!(
        md.contains("Deep"),
        "HTML strip should preserve text: {md:?}"
    );
    assert!(
        md.contains("body"),
        "HTML strip should preserve text: {md:?}"
    );
    assert!(
        !md.contains("<p>"),
        "raw html should not leak through: {md:?}"
    );
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
fn oversized_html_is_truncated_before_processing() {
    let cap = super::MAX_GMAIL_HTML_BODY_BYTES;
    let filler = "x".repeat(600 * 1024);
    let html =
        format!("<html><body><p>HEAD_MARKER</p>{filler}<p>TAIL_NEVER_SEEN</p></body></html>");
    assert!(html.len() > cap);
    let md = html_email_to_markdown(&html);
    assert!(md.contains("HEAD_MARKER"), "{md:?}");
    assert!(
        !md.contains("TAIL_NEVER_SEEN"),
        "tail past cap must not be processed: {md:?}"
    );
    assert!(
        md.contains("[Email HTML body truncated for processing]"),
        "expected truncation note: {md:?}"
    );
}

#[test]
fn truncated_all_whitespace_html_still_emits_truncation_note() {
    let cap = super::MAX_GMAIL_HTML_BODY_BYTES;
    let html = " ".repeat(cap + 10_000);
    assert!(html.len() > cap);
    let md = html_email_to_markdown(&html);
    assert_eq!(
        md, "[Email HTML body truncated for processing]",
        "empty body after strip must still signal truncation: {md:?}"
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
fn prefers_backend_markdown_formatted_when_present() {
    // Composio backend (tinyhumansai/backend#683 +) ships
    // `markdownFormatted` already URL-shortened + footer-stripped on
    // each message. When present, our post-processor must use it
    // verbatim instead of re-decoding the MIME tree.
    let mut v = json!({
        "messages": [{
            "messageId": "m1",
            "threadId": "t1",
            "subject": "s",
            "sender": "a@x.com",
            "to": "b@y.com",
            "messageTimestamp": "2026-04-17",
            "labelIds": [],
            // markdownFormatted should win — `messageText` would
            // otherwise be picked up as the fallback.
            "markdownFormatted": "# Already nice\n\nShort URL: https://gh.io/abc",
            "messageText": "<html>fallback should not be used</html>",
            "payload": {}
        }]
    });
    post_process("GMAIL_FETCH_EMAILS", None, &mut v);
    let md = v["messages"][0]["markdown"].as_str().unwrap();
    assert_eq!(md, "# Already nice\n\nShort URL: https://gh.io/abc");
    // Sanity: the fallback path was *not* taken.
    assert!(!md.contains("fallback should not be used"));
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
    // 3 messages expected but split yields a different number — the
    // format isn't recognized; caller falls back to MIME decode.
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
    // Backend marker yields the right count but the segments don't
    // mention the subjects from the JSON — alignment must be wrong,
    // so we refuse the split rather than ship a swapped raw archive.
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
    let slices =
        super::split_response_markdown_per_message_with_hint(md, 2, Some(&hints)).unwrap();
    assert_eq!(slices.len(), 2);
    assert!(slices[0].contains("Welcome to Gmail"));
    assert!(slices[1].contains("Your invoice"));
}

#[test]
fn split_with_hint_skips_messages_with_blank_subject() {
    // Notification mails with empty subjects should not block the
    // split — only non-empty subjects gate validation.
    let md = "## A\nbody1\n---\n## B\nbody2";
    let hints = vec![json!({"subject": "A"}), json!({"subject": ""})];
    let slices =
        super::split_response_markdown_per_message_with_hint(md, 2, Some(&hints)).unwrap();
    assert_eq!(slices.len(), 2);
}

#[test]
fn apply_response_level_markdown_stashes_per_message_field() {
    // Integration: a real-shape response with N messages + a
    // top-level markdownFormatted gets per-message slices stashed
    // onto each message map, ready for `extract_markdown_body` to
    // pick up.
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
    assert!(m1.contains("https://gh.io/abc"), "shortened URL must survive");
    assert!(m2.contains("World"));
    assert!(!m1.contains("World"), "no cross-message bleed");
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
fn fast_html_strip_handles_long_tags() {
    let long_href = format!(
        "<a href=\"https://example.com/{}\">Click me</a><p>After link</p>",
        "x".repeat(600)
    );
    let md = html_email_to_markdown(&long_href);
    assert!(md.contains("Click me"));
    assert!(md.contains("After link"));
}

#[test]
fn text_plain_attachment_does_not_outrank_html_body() {
    // multipart/mixed email with:
    //   - multipart/alternative (real body: text/plain + text/html)
    //   - text/plain attachment ("notes.txt")
    //
    // Without filtering attachments, find_decoded_part(_, "text/plain")
    // could pick up the attachment's content and return it instead of
    // the body. This test pins the attachment-skip behaviour.
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
                            {
                                "mimeType": "text/plain",
                                "body": { "data": b64("real body content") }
                            },
                            {
                                "mimeType": "text/html",
                                "body": { "data": b64("<p>real body content</p>") }
                            }
                        ]
                    },
                    {
                        "mimeType": "text/plain",
                        "filename": "notes.txt",
                        "body": { "data": b64("attachment content — not the body") }
                    }
                ]
            }
        }]
    });
    post_process("GMAIL_FETCH_EMAILS", None, &mut v);
    let md = v["messages"][0]["markdown"].as_str().unwrap();
    assert!(
        md.contains("real body content"),
        "real body should win, got: {md:?}"
    );
    assert!(
        !md.contains("attachment content"),
        "attachment must not leak into markdown body: {md:?}"
    );
}

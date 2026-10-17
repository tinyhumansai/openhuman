//! Round21 raw integration coverage for channels provider seams.
//!
//! Parser fixtures only: no real IMAP or SMTP traffic is performed.

use tinychannels::providers::email_channel::{
    test_support as email_support, EmailChannel, EmailConfig,
};

#[test]
fn email_parser_support_covers_text_html_attachment_and_message_building() {
    let text_raw = b"From: Alice <alice@example.com>\r\nSubject: Plain\r\nMessage-ID: <plain-1@example.com>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello from plain text.\r\n";
    let parsed = email_support::parse_email_fixture(text_raw).expect("plain parse");
    assert_eq!(parsed.sender, "alice@example.com");
    assert_eq!(parsed.subject.as_deref(), Some("Plain"));
    assert!(parsed.text.contains("Hello from plain text."));

    let html_raw = b"From: Bob <bob@example.com>\r\nSubject: HTML\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<div>Hello <strong>HTML</strong> body</div>\r\n";
    let parsed_html = email_support::parse_email_fixture(html_raw).expect("html parse");
    assert_eq!(parsed_html.sender, "bob@example.com");
    assert_eq!(parsed_html.text, "Hello HTML body");

    let attachment_raw = b"From: Ops <ops@example.com>\r\nSubject: Attachment\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Disposition: attachment; filename=\"note.txt\"\r\n\r\nattachment text body\r\n--b--\r\n";
    let parsed_attachment =
        email_support::parse_email_fixture(attachment_raw).expect("attachment parse");
    assert!(parsed_attachment.text.contains("[Attachment: note.txt]"));
    assert!(parsed_attachment.text.contains("attachment text body"));

    let channel = EmailChannel::new(EmailConfig {
        from_address: "bot@example.com".into(),
        allowed_senders: vec!["@example.com".into(), "trusted.test".into()],
        ..Default::default()
    });
    assert!(channel.is_sender_allowed("ALICE@example.com"));
    assert!(channel.is_sender_allowed("person@trusted.test"));
    assert!(!channel.is_sender_allowed("person@untrusted.test"));

    let message = channel
        .build_plain_message("listener@example.com", "Round21", "coverage body")
        .expect("build message");
    let wire = String::from_utf8_lossy(&message.formatted()).to_string();
    assert!(wire.contains("Subject: Round21"));
    assert!(wire.contains("coverage body"));
}

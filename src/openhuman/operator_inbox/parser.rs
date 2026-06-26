//! Real email parsing using the `mail-parser` crate.
//!
//! Extracts structured data from raw RFC 5322 email messages:
//! sender, subject, body, threading info, attachments.
//!
//! ## Log prefix
//!
//! `[operator-inbox-parser]`

use mail_parser::{MessageParser, MimeHeaders};
use tracing::debug;

/// Parsed email with structured fields extracted from raw RFC 5322.
#[derive(Debug, Clone)]
pub struct ParsedEmail {
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub subject: String,
    pub from: String,
    pub to: Vec<String>,
    pub date: Option<i64>,
    pub body_text: String,
    pub body_html: Option<String>,
    pub attachments: Vec<AttachmentInfo>,
    pub is_reply: bool,
}

/// Attachment metadata.
#[derive(Debug, Clone)]
pub struct AttachmentInfo {
    pub filename: String,
    pub content_type: String,
    pub size_bytes: usize,
}

/// Parse a raw RFC 5322 email message into structured fields.
pub fn parse_raw_email(raw: &[u8]) -> Option<ParsedEmail> {
    let message = MessageParser::default().parse(raw)?;

    let from = message
        .from()
        .and_then(|addrs| addrs.first())
        .map(|a| {
            if let Some(name) = a.name() {
                format!("{} <{}>", name, a.address().unwrap_or_default())
            } else {
                a.address().unwrap_or_default().to_string()
            }
        })
        .unwrap_or_default();

    let to: Vec<String> = message
        .to()
        .map(|addrs| {
            addrs
                .iter()
                .filter_map(|a| a.address().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let subject = message.subject().unwrap_or("(no subject)").to_string();
    let message_id = message.message_id().map(|s| s.to_string());
    let in_reply_to = message.in_reply_to().as_text().map(|s| s.to_string());

    let body_text = message
        .body_text(0)
        .map(|s| s.to_string())
        .unwrap_or_default();

    let body_html = message.body_html(0).map(|s| s.to_string());
    let date = message.date().map(|d| d.to_timestamp());

    let mut attachments = Vec::new();
    for part in message.attachments() {
        let part: &mail_parser::MessagePart = part;
        let ct = MimeHeaders::content_type(part);
        let content_type = ct
            .map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or("octet-stream")))
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let filename = MimeHeaders::content_disposition(part)
            .and_then(|d| d.attribute("filename"))
            .unwrap_or("unnamed")
            .to_string();
        attachments.push(AttachmentInfo {
            filename,
            content_type,
            size_bytes: part.len(),
        });
    }

    let is_reply =
        in_reply_to.is_some() || subject.starts_with("Re:") || subject.starts_with("RE:");

    debug!(
        from = %from,
        subject = %subject,
        attachments = attachments.len(),
        is_reply = is_reply,
        "[operator-inbox-parser] email parsed"
    );

    Some(ParsedEmail {
        message_id,
        in_reply_to,
        from,
        to,
        subject,
        date,
        body_text,
        body_html,
        attachments,
        is_reply,
    })
}

/// Extract urgency signals from a parsed email for priority scoring.
pub fn extract_urgency_signals(email: &ParsedEmail) -> UrgencySignals {
    let text = format!("{} {}", email.subject, email.body_text).to_lowercase();

    UrgencySignals {
        has_urgent_keywords: text.contains("urgent")
            || text.contains("emergency")
            || text.contains("critical")
            || text.contains("asap"),
        has_deadline: text.contains("deadline")
            || text.contains("by end of day")
            || text.contains("by eod")
            || text.contains("by tomorrow"),
        has_question: text.contains('?'),
        is_thread_reply: email.is_reply,
        has_attachments: !email.attachments.is_empty(),
        body_length: email.body_text.len(),
    }
}

/// Urgency signals extracted from email content.
#[derive(Debug, Clone)]
pub struct UrgencySignals {
    pub has_urgent_keywords: bool,
    pub has_deadline: bool,
    pub has_question: bool,
    pub is_thread_reply: bool,
    pub has_attachments: bool,
    pub body_length: usize,
}

impl UrgencySignals {
    /// Compute a priority score from 0.0 (low) to 1.0 (urgent).
    pub fn priority_score(&self) -> f64 {
        let mut score: f64 = 0.0;
        if self.has_urgent_keywords {
            score += 0.4;
        }
        if self.has_deadline {
            score += 0.25;
        }
        if self.is_thread_reply {
            score += 0.15;
        }
        if self.has_question {
            score += 0.1;
        }
        if self.has_attachments {
            score += 0.05;
        }
        if self.body_length > 500 {
            score += 0.05;
        }
        score.min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE_EMAIL: &[u8] = b"From: Alice <alice@example.com>\r\n\
        To: bob@example.com\r\n\
        Subject: Meeting tomorrow\r\n\
        Message-ID: <msg001@example.com>\r\n\
        Date: Wed, 20 May 2026 10:00:00 +0000\r\n\
        Content-Type: text/plain\r\n\
        \r\n\
        Hi Bob,\r\n\
        Can we meet tomorrow at 3pm?\r\n\
        Thanks, Alice\r\n";

    const REPLY_EMAIL: &[u8] = b"From: Bob <bob@example.com>\r\n\
        To: alice@example.com\r\n\
        Subject: Re: Meeting tomorrow\r\n\
        Message-ID: <msg002@example.com>\r\n\
        In-Reply-To: <msg001@example.com>\r\n\
        Content-Type: text/plain\r\n\
        \r\n\
        Sure, 3pm works for me.\r\n";

    const URGENT_EMAIL: &[u8] = b"From: boss@company.com\r\n\
        To: team@company.com\r\n\
        Subject: URGENT: Server down - need fix ASAP\r\n\
        Content-Type: text/plain\r\n\
        \r\n\
        The production server is down. This is critical.\r\n\
        We need this fixed immediately. Deadline is by end of day.\r\n";

    #[test]
    fn parse_simple_email() {
        let parsed = parse_raw_email(SIMPLE_EMAIL).unwrap();
        assert!(parsed.from.contains("alice@example.com"));
        assert_eq!(parsed.subject, "Meeting tomorrow");
        assert!(parsed.body_text.contains("Can we meet"));
        assert!(!parsed.is_reply);
    }

    #[test]
    fn parse_reply_detected() {
        let parsed = parse_raw_email(REPLY_EMAIL).unwrap();
        assert!(parsed.is_reply);
        assert!(parsed.in_reply_to.is_some());
    }

    #[test]
    fn urgent_email_high_priority() {
        let parsed = parse_raw_email(URGENT_EMAIL).unwrap();
        let signals = extract_urgency_signals(&parsed);
        assert!(signals.has_urgent_keywords);
        assert!(signals.has_deadline);
        assert!(signals.priority_score() > 0.6);
    }

    #[test]
    fn normal_email_low_priority() {
        let parsed = parse_raw_email(SIMPLE_EMAIL).unwrap();
        let signals = extract_urgency_signals(&parsed);
        assert!(!signals.has_urgent_keywords);
        assert!(signals.priority_score() < 0.3);
    }

    #[test]
    fn empty_returns_none() {
        assert!(parse_raw_email(b"").is_none());
    }

    #[test]
    fn priority_score_capped() {
        let signals = UrgencySignals {
            has_urgent_keywords: true,
            has_deadline: true,
            has_question: true,
            is_thread_reply: true,
            has_attachments: true,
            body_length: 1000,
        };
        assert!(signals.priority_score() <= 1.0);
    }
}

//! IMAP/SMTP types and algorithms for operator inbox.
//!
//! Config types, fetched-message types, JWZ threading, LLM prompt builders,
//! validation, and deadline extraction.
//!
//! Live IMAP fetching and SMTP sending live in `connection.rs`; background
//! polling lives in `poller.rs`.
//!
//! ## Log prefix
//!
//! `[operator-inbox-imap]`

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// IMAP connection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    /// Plaintext password (or app-specific password). Callers are responsible
    /// for decrypting before constructing this config.
    pub password: String,
    pub use_tls: bool,
    pub mailbox: String,
    /// OAuth2 token (for Gmail/Outlook).
    pub oauth2_token: Option<String>,
}

/// SMTP configuration for sending replies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub use_tls: bool,
    pub from_address: String,
    pub from_name: String,
}

/// A fetched email message (parsed from IMAP).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchedEmail {
    pub uid: u32,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub from: String,
    pub to: Vec<String>,
    pub subject: String,
    pub date: Option<String>,
    pub body_text: String,
    pub body_html: Option<String>,
    pub attachments: Vec<AttachmentInfo>,
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentInfo {
    pub filename: String,
    pub content_type: String,
    pub size_bytes: usize,
}

/// Email thread built using JWZ algorithm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailThread {
    pub thread_id: String,
    pub subject: String,
    pub messages: Vec<ThreadMessage>,
    pub participant_count: usize,
    pub last_activity: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadMessage {
    pub message_id: String,
    pub from: String,
    pub date: Option<String>,
    pub body_preview: String,
    pub depth: usize,
}

/// JWZ threading algorithm (simplified).
///
/// Groups emails into threads using Message-ID, In-Reply-To, and References headers.
/// This is the same algorithm used by Thunderbird, Gmail, and most email clients.
pub fn build_threads(emails: &[FetchedEmail]) -> Vec<EmailThread> {
    use std::collections::HashMap;

    // Step 1: Build ID table (message_id → email index).
    let mut id_table: HashMap<String, usize> = HashMap::new();
    for (idx, email) in emails.iter().enumerate() {
        if let Some(ref mid) = email.message_id {
            id_table.insert(mid.clone(), idx);
        }
    }

    // Step 2: Build parent-child relationships.
    let mut parent_of: HashMap<usize, usize> = HashMap::new();
    for (idx, email) in emails.iter().enumerate() {
        // Check In-Reply-To first.
        if let Some(ref irt) = email.in_reply_to {
            if let Some(&parent_idx) = id_table.get(irt) {
                if parent_idx != idx {
                    parent_of.insert(idx, parent_idx);
                    continue;
                }
            }
        }
        // Fall back to last Reference.
        if let Some(last_ref) = email.references.last() {
            if let Some(&parent_idx) = id_table.get(last_ref) {
                if parent_idx != idx {
                    parent_of.insert(idx, parent_idx);
                }
            }
        }
    }

    // Step 3: Find root messages (no parent).
    let mut roots: Vec<usize> = Vec::new();
    for idx in 0..emails.len() {
        if !parent_of.contains_key(&idx) {
            roots.push(idx);
        }
    }

    // Step 4: Build threads from roots.
    let mut threads = Vec::new();
    for root_idx in roots {
        let root = &emails[root_idx];
        let mut messages = Vec::new();
        let mut participants = std::collections::HashSet::new();

        // BFS to collect all messages in this thread.
        let mut queue = vec![(root_idx, 0usize)];
        while let Some((idx, depth)) = queue.pop() {
            let email = &emails[idx];
            participants.insert(email.from.clone());
            messages.push(ThreadMessage {
                message_id: email.message_id.clone().unwrap_or_default(),
                from: email.from.clone(),
                date: email.date.clone(),
                body_preview: email.body_text.chars().take(100).collect(),
                depth,
            });

            // Find children of this message.
            for (child_idx, parent_idx) in &parent_of {
                if *parent_idx == idx {
                    queue.push((*child_idx, depth + 1));
                }
            }
        }

        let thread_id = root
            .message_id
            .clone()
            .unwrap_or_else(|| format!("thread-{root_idx}"));

        threads.push(EmailThread {
            thread_id,
            subject: root.subject.clone(),
            messages,
            participant_count: participants.len(),
            last_activity: root.date.clone(),
        });
    }

    info!(
        thread_count = threads.len(),
        email_count = emails.len(),
        "[operator-inbox-imap] threads built"
    );
    threads
}

/// Build an LLM prompt for email priority classification.
pub fn build_priority_prompt(email: &FetchedEmail) -> String {
    format!(
        r#"Classify this email's priority. Respond with ONLY one word: urgent, high, normal, or low.

From: {}
Subject: {}
Body (first 500 chars): {}

Classification:"#,
        email.from,
        email.subject,
        crate::openhuman::util::utf8_safe_prefix_at_byte_boundary(&email.body_text, 500)
    )
}

/// Build an LLM prompt for generating a reply draft.
pub fn build_draft_prompt(email: &FetchedEmail, tone: &str, context: Option<&str>) -> String {
    let context_section = context
        .map(|c| format!("\nAdditional context: {c}\n"))
        .unwrap_or_default();

    format!(
        r#"Write a reply to this email in a {tone} tone. Be concise and professional.
{context_section}
Original email:
From: {}
Subject: {}
Body: {}

Reply (do not include subject line or headers, just the body):"#,
        email.from,
        email.subject,
        crate::openhuman::util::utf8_safe_prefix_at_byte_boundary(&email.body_text, 1000)
    )
}

/// Parse an LLM priority classification response.
pub fn parse_priority_response(response: &str) -> &'static str {
    let lower = response.trim().to_lowercase();
    if lower.contains("urgent") {
        "urgent"
    } else if lower.contains("high") {
        "high"
    } else if lower.contains("low") {
        "low"
    } else {
        "normal"
    }
}

/// Validate IMAP config has required fields.
pub fn validate_imap_config(config: &ImapConfig) -> Result<(), String> {
    if config.host.is_empty() {
        return Err("IMAP host is required".into());
    }
    if config.username.is_empty() {
        return Err("IMAP username is required".into());
    }
    if config.password.is_empty() && config.oauth2_token.is_none() {
        return Err("Either password or OAuth2 token is required".into());
    }
    if config.port == 0 {
        return Err("IMAP port must be non-zero".into());
    }
    debug!(
        host = %config.host,
        port = config.port,
        "[operator-inbox-imap] config validated"
    );
    Ok(())
}

/// Validate SMTP config.
pub fn validate_smtp_config(config: &SmtpConfig) -> Result<(), String> {
    if config.host.is_empty() {
        return Err("SMTP host is required".into());
    }
    if config.from_address.is_empty() {
        return Err("From address is required".into());
    }
    if !config.from_address.contains('@') {
        return Err("From address must be a valid email".into());
    }
    Ok(())
}

/// Extract follow-up deadline from email body.
///
/// Looks for patterns like "by Friday", "by end of week", "within 24 hours".
pub fn extract_followup_deadline(body: &str) -> Option<String> {
    let lower = body.to_lowercase();
    let patterns = [
        "by friday",
        "by monday",
        "by tuesday",
        "by wednesday",
        "by thursday",
        "by end of week",
        "by end of day",
        "by eod",
        "by eow",
        "within 24 hours",
        "within 48 hours",
        "asap",
        "urgent",
        "deadline",
    ];
    for pattern in &patterns {
        if lower.contains(pattern) {
            debug!(
                pattern = %pattern,
                "[operator-inbox-imap] follow-up deadline detected"
            );
            return Some(pattern.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_emails() -> Vec<FetchedEmail> {
        vec![
            FetchedEmail {
                uid: 1,
                message_id: Some("<msg1@example.com>".into()),
                in_reply_to: None,
                references: vec![],
                from: "alice@example.com".into(),
                to: vec!["bob@example.com".into()],
                subject: "Project update".into(),
                date: Some("2026-05-20T10:00:00Z".into()),
                body_text: "Here's the latest update on the project.".into(),
                body_html: None,
                attachments: vec![],
                flags: vec!["\\Seen".into()],
            },
            FetchedEmail {
                uid: 2,
                message_id: Some("<msg2@example.com>".into()),
                in_reply_to: Some("<msg1@example.com>".into()),
                references: vec!["<msg1@example.com>".into()],
                from: "bob@example.com".into(),
                to: vec!["alice@example.com".into()],
                subject: "Re: Project update".into(),
                date: Some("2026-05-20T11:00:00Z".into()),
                body_text: "Thanks for the update. Can we discuss by Friday?".into(),
                body_html: None,
                attachments: vec![],
                flags: vec![],
            },
            FetchedEmail {
                uid: 3,
                message_id: Some("<msg3@example.com>".into()),
                in_reply_to: Some("<msg2@example.com>".into()),
                references: vec!["<msg1@example.com>".into(), "<msg2@example.com>".into()],
                from: "alice@example.com".into(),
                to: vec!["bob@example.com".into()],
                subject: "Re: Project update".into(),
                date: Some("2026-05-20T12:00:00Z".into()),
                body_text: "Sure, let's meet Thursday.".into(),
                body_html: None,
                attachments: vec![],
                flags: vec![],
            },
            FetchedEmail {
                uid: 4,
                message_id: Some("<msg4@example.com>".into()),
                in_reply_to: None,
                references: vec![],
                from: "carol@example.com".into(),
                to: vec!["bob@example.com".into()],
                subject: "Unrelated topic".into(),
                date: Some("2026-05-20T09:00:00Z".into()),
                body_text: "Hey, quick question about the budget.".into(),
                body_html: None,
                attachments: vec![],
                flags: vec![],
            },
        ]
    }

    #[test]
    fn build_threads_groups_correctly() {
        let emails = sample_emails();
        let threads = build_threads(&emails);
        // Should produce 2 threads: one with 3 messages, one with 1.
        assert_eq!(threads.len(), 2);
        let project_thread = threads
            .iter()
            .find(|t| t.subject == "Project update")
            .unwrap();
        assert_eq!(project_thread.messages.len(), 3);
        assert_eq!(project_thread.participant_count, 2); // alice + bob
    }

    #[test]
    fn build_threads_single_email() {
        let emails = vec![sample_emails().remove(3)]; // "Unrelated topic"
        let threads = build_threads(&emails);
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].messages.len(), 1);
    }

    #[test]
    fn build_threads_empty() {
        let threads = build_threads(&[]);
        assert!(threads.is_empty());
    }

    #[test]
    fn build_threads_depth_tracking() {
        let emails = sample_emails();
        let threads = build_threads(&emails);
        let project_thread = threads
            .iter()
            .find(|t| t.subject == "Project update")
            .unwrap();
        // Root should be depth 0.
        assert_eq!(project_thread.messages[0].depth, 0);
    }

    #[test]
    fn validate_imap_config_valid() {
        let config = ImapConfig {
            host: "imap.gmail.com".into(),
            port: 993,
            username: "user@gmail.com".into(),
            password: "my_pass".into(),
            use_tls: true,
            mailbox: "INBOX".into(),
            oauth2_token: None,
        };
        assert!(validate_imap_config(&config).is_ok());
    }

    #[test]
    fn validate_imap_config_empty_host() {
        let config = ImapConfig {
            host: "".into(),
            port: 993,
            username: "user".into(),
            password: "pass".into(),
            use_tls: true,
            mailbox: "INBOX".into(),
            oauth2_token: None,
        };
        assert!(validate_imap_config(&config).is_err());
    }

    #[test]
    fn validate_imap_config_no_auth() {
        let config = ImapConfig {
            host: "imap.example.com".into(),
            port: 993,
            username: "user".into(),
            password: "".into(),
            use_tls: true,
            mailbox: "INBOX".into(),
            oauth2_token: None,
        };
        assert!(validate_imap_config(&config).is_err());
    }

    #[test]
    fn validate_imap_config_oauth2_sufficient() {
        let config = ImapConfig {
            host: "imap.gmail.com".into(),
            port: 993,
            username: "user@gmail.com".into(),
            password: "".into(),
            use_tls: true,
            mailbox: "INBOX".into(),
            oauth2_token: Some("ya29.token".into()),
        };
        assert!(validate_imap_config(&config).is_ok());
    }

    #[test]
    fn validate_smtp_config_valid() {
        let config = SmtpConfig {
            host: "smtp.gmail.com".into(),
            port: 587,
            username: "user".into(),
            password: "pass".into(),
            use_tls: true,
            from_address: "user@gmail.com".into(),
            from_name: "User".into(),
        };
        assert!(validate_smtp_config(&config).is_ok());
    }

    #[test]
    fn validate_smtp_config_invalid_email() {
        let config = SmtpConfig {
            host: "smtp.example.com".into(),
            port: 587,
            username: "user".into(),
            password: "pass".into(),
            use_tls: true,
            from_address: "not-an-email".into(),
            from_name: "User".into(),
        };
        assert!(validate_smtp_config(&config).is_err());
    }

    #[test]
    fn build_priority_prompt_includes_email_data() {
        let email = &sample_emails()[0];
        let prompt = build_priority_prompt(email);
        assert!(prompt.contains("alice@example.com"));
        assert!(prompt.contains("Project update"));
    }

    #[test]
    fn build_draft_prompt_includes_tone() {
        let email = &sample_emails()[0];
        let prompt = build_draft_prompt(email, "professional", None);
        assert!(prompt.contains("professional"));
        assert!(prompt.contains("Project update"));
    }

    #[test]
    fn build_draft_prompt_with_context() {
        let email = &sample_emails()[0];
        let prompt = build_draft_prompt(email, "casual", Some("We met last week"));
        assert!(prompt.contains("We met last week"));
    }

    #[test]
    fn parse_priority_response_variants() {
        assert_eq!(parse_priority_response("urgent"), "urgent");
        assert_eq!(parse_priority_response("HIGH"), "high");
        assert_eq!(parse_priority_response("This is low priority"), "low");
        assert_eq!(parse_priority_response("something else"), "normal");
    }

    #[test]
    fn extract_followup_deadline_found() {
        assert_eq!(
            extract_followup_deadline("Can we discuss by Friday?"),
            Some("by friday".into())
        );
        assert_eq!(
            extract_followup_deadline("Need this within 24 hours"),
            Some("within 24 hours".into())
        );
    }

    #[test]
    fn extract_followup_deadline_not_found() {
        assert_eq!(extract_followup_deadline("Just a casual hello"), None);
    }
}

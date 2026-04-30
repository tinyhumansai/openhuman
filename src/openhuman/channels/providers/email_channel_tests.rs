use super::*;

#[test]
fn default_smtp_port_uses_tls_port() {
    assert_eq!(default_smtp_port(), 465);
}

#[test]
fn email_config_default_uses_tls_smtp_defaults() {
    let config = EmailConfig::default();
    assert_eq!(config.smtp_port, 465);
    assert!(config.smtp_tls);
}

#[test]
fn default_idle_timeout_is_29_minutes() {
    assert_eq!(default_idle_timeout(), 1740);
}

#[tokio::test]
async fn seen_messages_starts_empty() {
    let channel = EmailChannel::new(EmailConfig::default());
    let seen = channel.seen_messages.lock().await;
    assert!(seen.is_empty());
}

#[tokio::test]
async fn seen_messages_tracks_unique_ids() {
    let channel = EmailChannel::new(EmailConfig::default());
    let mut seen = channel.seen_messages.lock().await;

    assert!(seen.insert("first-id".to_string()));
    assert!(!seen.insert("first-id".to_string()));
    assert!(seen.insert("second-id".to_string()));
    assert_eq!(seen.len(), 2);
}

// EmailConfig tests

#[test]
fn email_config_default() {
    let config = EmailConfig::default();
    assert_eq!(config.imap_host, "");
    assert_eq!(config.imap_port, 993);
    assert_eq!(config.imap_folder, "INBOX");
    assert_eq!(config.smtp_host, "");
    assert_eq!(config.smtp_port, 465);
    assert!(config.smtp_tls);
    assert_eq!(config.username, "");
    assert_eq!(config.password, "");
    assert_eq!(config.from_address, "");
    assert_eq!(config.idle_timeout_secs, 1740);
    assert!(config.allowed_senders.is_empty());
}

#[test]
fn email_config_custom() {
    let config = EmailConfig {
        imap_host: "imap.example.com".to_string(),
        imap_port: 993,
        imap_folder: "Archive".to_string(),
        smtp_host: "smtp.example.com".to_string(),
        smtp_port: 465,
        smtp_tls: true,
        username: "user@example.com".to_string(),
        password: "pass123".to_string(),
        from_address: "bot@example.com".to_string(),
        idle_timeout_secs: 1200,
        allowed_senders: vec!["allowed@example.com".to_string()],
    };
    assert_eq!(config.imap_host, "imap.example.com");
    assert_eq!(config.imap_folder, "Archive");
    assert_eq!(config.idle_timeout_secs, 1200);
}

#[test]
fn email_config_clone() {
    let config = EmailConfig {
        imap_host: "imap.test.com".to_string(),
        imap_port: 993,
        imap_folder: "INBOX".to_string(),
        smtp_host: "smtp.test.com".to_string(),
        smtp_port: 587,
        smtp_tls: true,
        username: "user@test.com".to_string(),
        password: "secret".to_string(),
        from_address: "bot@test.com".to_string(),
        idle_timeout_secs: 1740,
        allowed_senders: vec!["*".to_string()],
    };
    let cloned = config.clone();
    assert_eq!(cloned.imap_host, config.imap_host);
    assert_eq!(cloned.smtp_port, config.smtp_port);
    assert_eq!(cloned.allowed_senders, config.allowed_senders);
}

// EmailChannel tests

#[tokio::test]
async fn email_channel_new() {
    let config = EmailConfig::default();
    let channel = EmailChannel::new(config.clone());
    assert_eq!(channel.config.imap_host, config.imap_host);

    let seen_guard = channel.seen_messages.lock().await;
    assert_eq!(seen_guard.len(), 0);
}

#[test]
fn email_channel_name() {
    let channel = EmailChannel::new(EmailConfig::default());
    assert_eq!(channel.name(), "email");
}

// is_sender_allowed tests

#[test]
fn is_sender_allowed_empty_list_denies_all() {
    let config = EmailConfig {
        allowed_senders: vec![],
        ..Default::default()
    };
    let channel = EmailChannel::new(config);
    assert!(!channel.is_sender_allowed("anyone@example.com"));
    assert!(!channel.is_sender_allowed("user@test.com"));
}

#[test]
fn is_sender_allowed_wildcard_allows_all() {
    let config = EmailConfig {
        allowed_senders: vec!["*".to_string()],
        ..Default::default()
    };
    let channel = EmailChannel::new(config);
    assert!(channel.is_sender_allowed("anyone@example.com"));
    assert!(channel.is_sender_allowed("user@test.com"));
    assert!(channel.is_sender_allowed("random@domain.org"));
}

#[test]
fn is_sender_allowed_specific_email() {
    let config = EmailConfig {
        allowed_senders: vec!["allowed@example.com".to_string()],
        ..Default::default()
    };
    let channel = EmailChannel::new(config);
    assert!(channel.is_sender_allowed("allowed@example.com"));
    assert!(!channel.is_sender_allowed("other@example.com"));
    assert!(!channel.is_sender_allowed("allowed@other.com"));
}

#[test]
fn is_sender_allowed_domain_with_at_prefix() {
    let config = EmailConfig {
        allowed_senders: vec!["@example.com".to_string()],
        ..Default::default()
    };
    let channel = EmailChannel::new(config);
    assert!(channel.is_sender_allowed("user@example.com"));
    assert!(channel.is_sender_allowed("admin@example.com"));
    assert!(!channel.is_sender_allowed("user@other.com"));
}

#[test]
fn is_sender_allowed_domain_without_at_prefix() {
    let config = EmailConfig {
        allowed_senders: vec!["example.com".to_string()],
        ..Default::default()
    };
    let channel = EmailChannel::new(config);
    assert!(channel.is_sender_allowed("user@example.com"));
    assert!(channel.is_sender_allowed("admin@example.com"));
    assert!(!channel.is_sender_allowed("user@other.com"));
}

#[test]
fn is_sender_allowed_case_insensitive() {
    let config = EmailConfig {
        allowed_senders: vec!["Allowed@Example.COM".to_string()],
        ..Default::default()
    };
    let channel = EmailChannel::new(config);
    assert!(channel.is_sender_allowed("allowed@example.com"));
    assert!(channel.is_sender_allowed("ALLOWED@EXAMPLE.COM"));
    assert!(channel.is_sender_allowed("AlLoWeD@eXaMpLe.cOm"));
}

#[test]
fn is_sender_allowed_multiple_senders() {
    let config = EmailConfig {
        allowed_senders: vec![
            "user1@example.com".to_string(),
            "user2@test.com".to_string(),
            "@allowed.com".to_string(),
        ],
        ..Default::default()
    };
    let channel = EmailChannel::new(config);
    assert!(channel.is_sender_allowed("user1@example.com"));
    assert!(channel.is_sender_allowed("user2@test.com"));
    assert!(channel.is_sender_allowed("anyone@allowed.com"));
    assert!(!channel.is_sender_allowed("user3@example.com"));
}

#[test]
fn is_sender_allowed_wildcard_with_specific() {
    let config = EmailConfig {
        allowed_senders: vec!["*".to_string(), "specific@example.com".to_string()],
        ..Default::default()
    };
    let channel = EmailChannel::new(config);
    assert!(channel.is_sender_allowed("anyone@example.com"));
    assert!(channel.is_sender_allowed("specific@example.com"));
}

#[test]
fn is_sender_allowed_empty_sender() {
    let config = EmailConfig {
        allowed_senders: vec!["@example.com".to_string()],
        ..Default::default()
    };
    let channel = EmailChannel::new(config);
    assert!(!channel.is_sender_allowed(""));
    // "@example.com" ends with "@example.com" so it's allowed
    assert!(channel.is_sender_allowed("@example.com"));
}

// strip_html tests

#[test]
fn strip_html_basic() {
    assert_eq!(EmailChannel::strip_html("<p>Hello</p>"), "Hello");
    assert_eq!(EmailChannel::strip_html("<div>World</div>"), "World");
}

#[test]
fn strip_html_nested_tags() {
    assert_eq!(
        EmailChannel::strip_html("<div><p>Hello <strong>World</strong></p></div>"),
        "Hello World"
    );
}

#[test]
fn strip_html_multiple_lines() {
    let html = "<div>\n  <p>Line 1</p>\n  <p>Line 2</p>\n</div>";
    assert_eq!(EmailChannel::strip_html(html), "Line 1 Line 2");
}

#[test]
fn strip_html_preserves_text() {
    assert_eq!(EmailChannel::strip_html("No tags here"), "No tags here");
    assert_eq!(EmailChannel::strip_html(""), "");
}

#[test]
fn strip_html_handles_malformed() {
    assert_eq!(EmailChannel::strip_html("<p>Unclosed"), "Unclosed");
    // The function removes everything between < and >, so "Text>with>brackets" becomes "Textwithbrackets"
    assert_eq!(
        EmailChannel::strip_html("Text>with>brackets"),
        "Textwithbrackets"
    );
}

#[test]
fn strip_html_self_closing_tags() {
    // Self-closing tags are removed but don't add spaces
    assert_eq!(EmailChannel::strip_html("Hello<br/>World"), "HelloWorld");
    assert_eq!(EmailChannel::strip_html("Text<hr/>More"), "TextMore");
}

#[test]
fn strip_html_attributes_preserved() {
    assert_eq!(
        EmailChannel::strip_html("<a href=\"http://example.com\">Link</a>"),
        "Link"
    );
}

#[test]
fn strip_html_multiple_spaces_collapsed() {
    assert_eq!(
        EmailChannel::strip_html("<p>Word</p>  <p>Word</p>"),
        "Word Word"
    );
}

#[test]
fn strip_html_special_characters() {
    assert_eq!(
        EmailChannel::strip_html("<span>&lt;tag&gt;</span>"),
        "&lt;tag&gt;"
    );
}

// Default function tests

#[test]
fn default_imap_port_returns_993() {
    assert_eq!(default_imap_port(), 993);
}

#[test]
fn default_smtp_port_returns_465() {
    assert_eq!(default_smtp_port(), 465);
}

#[test]
fn default_imap_folder_returns_inbox() {
    assert_eq!(default_imap_folder(), "INBOX");
}

#[test]
fn default_true_returns_true() {
    assert!(default_true());
}

// EmailConfig serialization tests

#[test]
fn email_config_serialize_deserialize() {
    let config = EmailConfig {
        imap_host: "imap.example.com".to_string(),
        imap_port: 993,
        imap_folder: "INBOX".to_string(),
        smtp_host: "smtp.example.com".to_string(),
        smtp_port: 587,
        smtp_tls: true,
        username: "user@example.com".to_string(),
        password: "password123".to_string(),
        from_address: "bot@example.com".to_string(),
        idle_timeout_secs: 1740,
        allowed_senders: vec!["allowed@example.com".to_string()],
    };

    let json = serde_json::to_string(&config).unwrap();
    let deserialized: EmailConfig = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.imap_host, config.imap_host);
    assert_eq!(deserialized.smtp_port, config.smtp_port);
    assert_eq!(deserialized.allowed_senders, config.allowed_senders);
}

#[test]
fn email_config_deserialize_with_defaults() {
    let json = r#"{
        "imap_host": "imap.test.com",
        "smtp_host": "smtp.test.com",
        "username": "user",
        "password": "pass",
        "from_address": "bot@test.com"
    }"#;

    let config: EmailConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.imap_port, 993); // default
    assert_eq!(config.smtp_port, 465); // default
    assert!(config.smtp_tls); // default
    assert_eq!(config.idle_timeout_secs, 1740); // default
}

#[test]
fn idle_timeout_deserializes_explicit_value() {
    let json = r#"{
        "imap_host": "imap.test.com",
        "smtp_host": "smtp.test.com",
        "username": "user",
        "password": "pass",
        "from_address": "bot@test.com",
        "idle_timeout_secs": 900
    }"#;
    let config: EmailConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.idle_timeout_secs, 900);
}

#[test]
fn idle_timeout_deserializes_legacy_poll_interval_alias() {
    let json = r#"{
        "imap_host": "imap.test.com",
        "smtp_host": "smtp.test.com",
        "username": "user",
        "password": "pass",
        "from_address": "bot@test.com",
        "poll_interval_secs": 120
    }"#;
    let config: EmailConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.idle_timeout_secs, 120);
}

#[test]
fn idle_timeout_propagates_to_channel() {
    let config = EmailConfig {
        idle_timeout_secs: 600,
        ..Default::default()
    };
    let channel = EmailChannel::new(config);
    assert_eq!(channel.config.idle_timeout_secs, 600);
}

#[test]
fn email_config_debug_output() {
    let config = EmailConfig {
        imap_host: "imap.debug.com".to_string(),
        ..Default::default()
    };
    let debug_str = format!("{:?}", config);
    assert!(debug_str.contains("imap.debug.com"));
}

// ── is_sender_allowed comprehensive matrix ─────────────────────

fn channel_with_allowlist(allowlist: Vec<String>) -> EmailChannel {
    let cfg = EmailConfig {
        imap_host: "imap.x".into(),
        imap_port: 993,
        imap_folder: "INBOX".into(),
        smtp_host: "smtp.x".into(),
        smtp_port: 465,
        smtp_tls: true,
        username: "u".into(),
        password: "p".into(),
        from_address: "me@x".into(),
        idle_timeout_secs: 300,
        allowed_senders: allowlist,
    };
    EmailChannel::new(cfg)
}

#[test]
fn is_sender_allowed_empty_denies_all() {
    let ch = channel_with_allowlist(vec![]);
    assert!(!ch.is_sender_allowed("anyone@any.com"));
}

#[test]
fn is_sender_allowed_wildcard_allows_everyone() {
    let ch = channel_with_allowlist(vec!["*".into()]);
    assert!(ch.is_sender_allowed("anyone@any.com"));
    assert!(ch.is_sender_allowed("other@different.com"));
}

#[test]
fn is_sender_allowed_full_email_exact_match_case_insensitive() {
    let ch = channel_with_allowlist(vec!["alice@example.com".into()]);
    assert!(ch.is_sender_allowed("alice@example.com"));
    assert!(ch.is_sender_allowed("ALICE@EXAMPLE.COM"));
    assert!(!ch.is_sender_allowed("bob@example.com"));
}

#[test]
fn is_sender_allowed_at_prefix_domain_match() {
    let ch = channel_with_allowlist(vec!["@trusted.com".into()]);
    assert!(ch.is_sender_allowed("user@trusted.com"));
    assert!(ch.is_sender_allowed("other@Trusted.com"));
    assert!(!ch.is_sender_allowed("user@untrusted.com"));
}

#[test]
fn is_sender_allowed_bare_domain_match_is_case_insensitive() {
    let ch = channel_with_allowlist(vec!["trusted.com".into()]);
    assert!(ch.is_sender_allowed("user@trusted.com"));
    assert!(ch.is_sender_allowed("USER@TRUSTED.COM"));
    assert!(!ch.is_sender_allowed("user@other.com"));
}

#[test]
fn is_sender_allowed_prevents_subdomain_confusion() {
    // "trusted.com" must NOT match "user@malicioustrusted.com"
    let ch = channel_with_allowlist(vec!["trusted.com".into()]);
    assert!(!ch.is_sender_allowed("user@notmytrusted.com"));
    assert!(!ch.is_sender_allowed("user@trusted.com.evil.com"));
}

// ── strip_html edge cases ──────────────────────────────────────

#[test]
fn strip_html_empty_string() {
    assert_eq!(EmailChannel::strip_html(""), "");
}

#[test]
fn strip_html_only_tags() {
    assert_eq!(EmailChannel::strip_html("<p></p><br/>"), "");
}

#[test]
fn strip_html_unclosed_tag_eats_rest_until_gt() {
    // A '<' without '>' enters tag mode; anything after until a '>' is
    // discarded. This is the implementation's behaviour — lock it in.
    assert_eq!(EmailChannel::strip_html("before<never closed"), "before");
}

#[test]
fn strip_html_collapses_whitespace_runs() {
    assert_eq!(
        EmailChannel::strip_html("<p>hello</p>\n\n\n   <p>world</p>"),
        "hello world"
    );
}

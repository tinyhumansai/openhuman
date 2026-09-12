use super::*;
use serde_json::json;

fn creds(v: Value) -> serde_json::Map<String, Value> {
    v.as_object().cloned().unwrap()
}

#[test]
fn build_email_config_applies_defaults() {
    let c = creds(json!({
        "imap_host": "imap.fastmail.com",
        "smtp_host": "smtp.fastmail.com",
        "username": "alice@example.com",
        "password": "app-pass",
    }));
    let cfg = build_email_config(&c, None).expect("should build");
    assert_eq!(cfg.imap_port, 993);
    assert_eq!(cfg.smtp_port, 465);
    assert!(cfg.smtp_tls);
    assert_eq!(cfg.imap_folder, "INBOX");
    // from_address defaults to the username when omitted.
    assert_eq!(cfg.from_address, "alice@example.com");
    // Absent allowlist defaults to allow-any so a fresh mailbox receives.
    assert_eq!(cfg.allowed_senders, vec!["*".to_string()]);
    assert_eq!(cfg.idle_timeout_secs, 1740);
}

#[test]
fn build_email_config_honors_explicit_values() {
    let c = creds(json!({
        "imap_host": "mail.self.host",
        "imap_port": "1993",
        "imap_folder": "Archive",
        "smtp_host": "mail.self.host",
        "smtp_port": "2465",
        "smtp_tls": "false",
        "username": "bob@self.host",
        "password": "secret",
        "from_address": "Bob <bob@self.host>",
        "allowed_senders": "@team.com, boss@corp.com , @team.com",
    }));
    let cfg = build_email_config(&c, None).expect("should build");
    assert_eq!(cfg.imap_port, 1993);
    assert_eq!(cfg.smtp_port, 2465);
    assert!(!cfg.smtp_tls);
    assert_eq!(cfg.imap_folder, "Archive");
    assert_eq!(cfg.from_address, "Bob <bob@self.host>");
    // '@'-domain syntax preserved; duplicate collapsed case-insensitively.
    assert_eq!(
        cfg.allowed_senders,
        vec!["@team.com".to_string(), "boss@corp.com".to_string()]
    );
}

#[test]
fn build_email_config_rejects_missing_required() {
    for missing in ["imap_host", "smtp_host", "username", "password"] {
        let mut obj = json!({
            "imap_host": "h",
            "smtp_host": "h",
            "username": "u",
            "password": "p",
        });
        obj.as_object_mut().unwrap().remove(missing);
        let err =
            build_email_config(&creds(obj), None).expect_err("must reject missing required field");
        assert!(err.contains(missing), "error should name {missing}: {err}");
    }
}

#[test]
fn build_email_config_preserves_existing_idle_timeout() {
    let existing = EmailConfig {
        idle_timeout_secs: 600,
        ..EmailConfig::default()
    };
    let c = creds(json!({
        "imap_host": "h", "smtp_host": "h", "username": "u", "password": "p",
    }));
    let cfg = build_email_config(&c, Some(&existing)).expect("should build");
    assert_eq!(cfg.idle_timeout_secs, 600);
}

#[test]
fn parse_port_field_variants() {
    let c = creds(json!({
        "p_str": "8143", "p_blank": "  ", "p_num": 143, "p_bad": "abc",
        "p_zero_str": "0", "p_zero_num": 0
    }));
    assert_eq!(parse_port_field(&c, "p_str", 993).unwrap(), 8143);
    assert_eq!(parse_port_field(&c, "p_blank", 993).unwrap(), 993);
    assert_eq!(parse_port_field(&c, "p_num", 993).unwrap(), 143);
    assert_eq!(parse_port_field(&c, "absent", 465).unwrap(), 465);
    assert!(parse_port_field(&c, "p_bad", 993).is_err());
    // Port 0 is the OS "any" sentinel, never valid for a mailbox.
    assert!(parse_port_field(&c, "p_zero_str", 993).is_err());
    assert!(parse_port_field(&c, "p_zero_num", 993).is_err());
}

#[test]
fn parse_email_senders_defaults_and_dedup() {
    // Absent → allow any.
    assert_eq!(parse_email_senders(None), vec!["*".to_string()]);
    // Blank string → allow any (never accidental deny-all).
    assert_eq!(
        parse_email_senders(Some(&json!("  "))),
        vec!["*".to_string()]
    );
    // Array form joins, preserves '@', dedups.
    assert_eq!(
        parse_email_senders(Some(&json!(["@x.com", "a@y.com", "@X.COM"]))),
        vec!["@x.com".to_string(), "a@y.com".to_string()]
    );
}

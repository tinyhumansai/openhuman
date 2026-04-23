use super::*;

// ── IRC message parsing ──────────────────────────────────

#[test]
fn parse_privmsg_with_prefix() {
    let msg = IrcMessage::parse(":nick!user@host PRIVMSG #channel :Hello world").unwrap();
    assert_eq!(msg.prefix.as_deref(), Some("nick!user@host"));
    assert_eq!(msg.command, "PRIVMSG");
    assert_eq!(msg.params, vec!["#channel", "Hello world"]);
}

#[test]
fn parse_privmsg_dm() {
    let msg = IrcMessage::parse(":alice!a@host PRIVMSG botname :hi there").unwrap();
    assert_eq!(msg.command, "PRIVMSG");
    assert_eq!(msg.params, vec!["botname", "hi there"]);
    assert_eq!(msg.nick(), Some("alice"));
}

#[test]
fn parse_ping() {
    let msg = IrcMessage::parse("PING :server.example.com").unwrap();
    assert!(msg.prefix.is_none());
    assert_eq!(msg.command, "PING");
    assert_eq!(msg.params, vec!["server.example.com"]);
}

#[test]
fn parse_numeric_reply() {
    let msg = IrcMessage::parse(":server 001 botname :Welcome to the IRC network").unwrap();
    assert_eq!(msg.prefix.as_deref(), Some("server"));
    assert_eq!(msg.command, "001");
    assert_eq!(msg.params, vec!["botname", "Welcome to the IRC network"]);
}

#[test]
fn parse_no_trailing() {
    let msg = IrcMessage::parse(":server 433 * botname").unwrap();
    assert_eq!(msg.command, "433");
    assert_eq!(msg.params, vec!["*", "botname"]);
}

#[test]
fn parse_cap_ack() {
    let msg = IrcMessage::parse(":server CAP * ACK :sasl").unwrap();
    assert_eq!(msg.command, "CAP");
    assert_eq!(msg.params, vec!["*", "ACK", "sasl"]);
}

#[test]
fn parse_empty_line_returns_none() {
    assert!(IrcMessage::parse("").is_none());
    assert!(IrcMessage::parse("\r\n").is_none());
}

#[test]
fn parse_strips_crlf() {
    let msg = IrcMessage::parse("PING :test\r\n").unwrap();
    assert_eq!(msg.params, vec!["test"]);
}

#[test]
fn parse_command_uppercase() {
    let msg = IrcMessage::parse("ping :test").unwrap();
    assert_eq!(msg.command, "PING");
}

#[test]
fn nick_extraction_full_prefix() {
    let msg = IrcMessage::parse(":nick!user@host PRIVMSG #ch :msg").unwrap();
    assert_eq!(msg.nick(), Some("nick"));
}

#[test]
fn nick_extraction_nick_only() {
    let msg = IrcMessage::parse(":server 001 bot :Welcome").unwrap();
    assert_eq!(msg.nick(), Some("server"));
}

#[test]
fn nick_extraction_no_prefix() {
    let msg = IrcMessage::parse("PING :token").unwrap();
    assert_eq!(msg.nick(), None);
}

#[test]
fn parse_authenticate_plus() {
    let msg = IrcMessage::parse("AUTHENTICATE +").unwrap();
    assert_eq!(msg.command, "AUTHENTICATE");
    assert_eq!(msg.params, vec!["+"]);
}

// ── SASL PLAIN encoding ─────────────────────────────────

#[test]
fn sasl_plain_encode() {
    let encoded = encode_sasl_plain("jilles", "sesame");
    // \0jilles\0sesame → base64
    assert_eq!(encoded, "AGppbGxlcwBzZXNhbWU=");
}

#[test]
fn sasl_plain_empty_password() {
    let encoded = encode_sasl_plain("nick", "");
    // \0nick\0 → base64
    assert_eq!(encoded, "AG5pY2sA");
}

// ── Message splitting ───────────────────────────────────

#[test]
fn split_short_message() {
    let chunks = split_message("hello", 400);
    assert_eq!(chunks, vec!["hello"]);
}

#[test]
fn split_long_message() {
    let msg = "a".repeat(800);
    let chunks = split_message(&msg, 400);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].len(), 400);
    assert_eq!(chunks[1].len(), 400);
}

#[test]
fn split_exact_boundary() {
    let msg = "a".repeat(400);
    let chunks = split_message(&msg, 400);
    assert_eq!(chunks.len(), 1);
}

#[test]
fn split_unicode_safe() {
    // 'é' is 2 bytes in UTF-8; splitting at byte 3 would split mid-char
    let msg = "ééé"; // 6 bytes
    let chunks = split_message(msg, 3);
    // Should split at char boundary (2 bytes), not mid-char
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0], "é");
    assert_eq!(chunks[1], "é");
    assert_eq!(chunks[2], "é");
}

#[test]
fn split_empty_message() {
    let chunks = split_message("", 400);
    assert_eq!(chunks, vec![""]);
}

#[test]
fn split_newlines_into_separate_lines() {
    let chunks = split_message("line one\nline two\nline three", 400);
    assert_eq!(chunks, vec!["line one", "line two", "line three"]);
}

#[test]
fn split_crlf_newlines() {
    let chunks = split_message("hello\r\nworld", 400);
    assert_eq!(chunks, vec!["hello", "world"]);
}

#[test]
fn split_skips_empty_lines() {
    let chunks = split_message("hello\n\n\nworld", 400);
    assert_eq!(chunks, vec!["hello", "world"]);
}

#[test]
fn split_trailing_newline() {
    let chunks = split_message("hello\n", 400);
    assert_eq!(chunks, vec!["hello"]);
}

#[test]
fn split_multiline_with_long_line() {
    let long = "a".repeat(800);
    let msg = format!("short\n{long}\nend");
    let chunks = split_message(&msg, 400);
    assert_eq!(chunks.len(), 4);
    assert_eq!(chunks[0], "short");
    assert_eq!(chunks[1].len(), 400);
    assert_eq!(chunks[2].len(), 400);
    assert_eq!(chunks[3], "end");
}

#[test]
fn split_only_newlines() {
    let chunks = split_message("\n\n\n", 400);
    assert_eq!(chunks, vec![""]);
}

// ── Allowlist ───────────────────────────────────────────

#[test]
fn wildcard_allows_anyone() {
    let ch = make_channel();
    // Default make_channel has wildcard
    assert!(ch.is_user_allowed("anyone"));
    assert!(ch.is_user_allowed("stranger"));
}

#[test]
fn specific_user_allowed() {
    let ch = IrcChannel::new(IrcChannelConfig {
        server: "irc.test".into(),
        port: 6697,
        nickname: "bot".into(),
        username: None,
        channels: vec![],
        allowed_users: vec!["alice".into(), "bob".into()],
        server_password: None,
        nickserv_password: None,
        sasl_password: None,
        verify_tls: true,
    });
    assert!(ch.is_user_allowed("alice"));
    assert!(ch.is_user_allowed("bob"));
    assert!(!ch.is_user_allowed("eve"));
}

#[test]
fn allowlist_case_insensitive() {
    let ch = IrcChannel::new(IrcChannelConfig {
        server: "irc.test".into(),
        port: 6697,
        nickname: "bot".into(),
        username: None,
        channels: vec![],
        allowed_users: vec!["Alice".into()],
        server_password: None,
        nickserv_password: None,
        sasl_password: None,
        verify_tls: true,
    });
    assert!(ch.is_user_allowed("alice"));
    assert!(ch.is_user_allowed("ALICE"));
    assert!(ch.is_user_allowed("Alice"));
}

#[test]
fn empty_allowlist_denies_all() {
    let ch = IrcChannel::new(IrcChannelConfig {
        server: "irc.test".into(),
        port: 6697,
        nickname: "bot".into(),
        username: None,
        channels: vec![],
        allowed_users: vec![],
        server_password: None,
        nickserv_password: None,
        sasl_password: None,
        verify_tls: true,
    });
    assert!(!ch.is_user_allowed("anyone"));
}

// ── Constructor ─────────────────────────────────────────

#[test]
fn new_defaults_username_to_nickname() {
    let ch = IrcChannel::new(IrcChannelConfig {
        server: "irc.test".into(),
        port: 6697,
        nickname: "mybot".into(),
        username: None,
        channels: vec![],
        allowed_users: vec![],
        server_password: None,
        nickserv_password: None,
        sasl_password: None,
        verify_tls: true,
    });
    assert_eq!(ch.username, "mybot");
}

#[test]
fn new_uses_explicit_username() {
    let ch = IrcChannel::new(IrcChannelConfig {
        server: "irc.test".into(),
        port: 6697,
        nickname: "mybot".into(),
        username: Some("customuser".into()),
        channels: vec![],
        allowed_users: vec![],
        server_password: None,
        nickserv_password: None,
        sasl_password: None,
        verify_tls: true,
    });
    assert_eq!(ch.username, "customuser");
    assert_eq!(ch.nickname, "mybot");
}

#[test]
fn name_returns_irc() {
    let ch = make_channel();
    assert_eq!(ch.name(), "irc");
}

#[test]
fn new_stores_all_fields() {
    let ch = IrcChannel::new(IrcChannelConfig {
        server: "irc.example.com".into(),
        port: 6697,
        nickname: "zcbot".into(),
        username: Some("openhuman".into()),
        channels: vec!["#test".into()],
        allowed_users: vec!["alice".into()],
        server_password: Some("serverpass".into()),
        nickserv_password: Some("nspass".into()),
        sasl_password: Some("saslpass".into()),
        verify_tls: false,
    });
    assert_eq!(ch.server, "irc.example.com");
    assert_eq!(ch.port, 6697);
    assert_eq!(ch.nickname, "zcbot");
    assert_eq!(ch.username, "openhuman");
    assert_eq!(ch.channels, vec!["#test"]);
    assert_eq!(ch.allowed_users, vec!["alice"]);
    assert_eq!(ch.server_password.as_deref(), Some("serverpass"));
    assert_eq!(ch.nickserv_password.as_deref(), Some("nspass"));
    assert_eq!(ch.sasl_password.as_deref(), Some("saslpass"));
    assert!(!ch.verify_tls);
}

// ── Config serde ────────────────────────────────────────

#[test]
fn irc_config_serde_roundtrip() {
    use crate::openhuman::config::schema::IrcConfig;

    let config = IrcConfig {
        server: "irc.example.com".into(),
        port: 6697,
        nickname: "zcbot".into(),
        username: Some("openhuman".into()),
        channels: vec!["#test".into(), "#dev".into()],
        allowed_users: vec!["alice".into()],
        server_password: None,
        nickserv_password: Some("secret".into()),
        sasl_password: None,
        verify_tls: Some(true),
    };

    let toml_str = toml::to_string(&config).unwrap();
    let parsed: IrcConfig = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.server, "irc.example.com");
    assert_eq!(parsed.port, 6697);
    assert_eq!(parsed.nickname, "zcbot");
    assert_eq!(parsed.username.as_deref(), Some("openhuman"));
    assert_eq!(parsed.channels, vec!["#test", "#dev"]);
    assert_eq!(parsed.allowed_users, vec!["alice"]);
    assert!(parsed.server_password.is_none());
    assert_eq!(parsed.nickserv_password.as_deref(), Some("secret"));
    assert!(parsed.sasl_password.is_none());
    assert_eq!(parsed.verify_tls, Some(true));
}

#[test]
fn irc_config_minimal_toml() {
    use crate::openhuman::config::schema::IrcConfig;

    let toml_str = r#"
server = "irc.example.com"
nickname = "bot"
"#;
    let parsed: IrcConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(parsed.server, "irc.example.com");
    assert_eq!(parsed.port, 6697); // default
    assert_eq!(parsed.nickname, "bot");
    assert!(parsed.username.is_none());
    assert!(parsed.channels.is_empty());
    assert!(parsed.allowed_users.is_empty());
    assert!(parsed.server_password.is_none());
    assert!(parsed.nickserv_password.is_none());
    assert!(parsed.sasl_password.is_none());
    assert!(parsed.verify_tls.is_none());
}

#[test]
fn irc_config_default_port() {
    use crate::openhuman::config::schema::IrcConfig;

    let json = r#"{"server":"irc.test","nickname":"bot"}"#;
    let parsed: IrcConfig = serde_json::from_str(json).unwrap();
    assert_eq!(parsed.port, 6697);
}

// ── Helpers ─────────────────────────────────────────────

fn make_channel() -> IrcChannel {
    IrcChannel::new(IrcChannelConfig {
        server: "irc.example.com".into(),
        port: 6697,
        nickname: "zcbot".into(),
        username: None,
        channels: vec!["#openhuman".into()],
        allowed_users: vec!["*".into()],
        server_password: None,
        nickserv_password: None,
        sasl_password: None,
        verify_tls: true,
    })
}

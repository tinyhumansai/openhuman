use super::*;

#[test]
fn creates_with_contacts() {
    let ch = IMessageChannel::new(vec!["+1234567890".into()]);
    assert_eq!(ch.allowed_contacts.len(), 1);
    assert_eq!(ch.poll_interval_secs, 3);
}

#[test]
fn creates_with_empty_contacts() {
    let ch = IMessageChannel::new(vec![]);
    assert!(ch.allowed_contacts.is_empty());
}

#[test]
fn wildcard_allows_anyone() {
    let ch = IMessageChannel::new(vec!["*".into()]);
    assert!(ch.is_contact_allowed("+1234567890"));
    assert!(ch.is_contact_allowed("random@icloud.com"));
    assert!(ch.is_contact_allowed(""));
}

#[test]
fn specific_contact_allowed() {
    let ch = IMessageChannel::new(vec!["+1234567890".into(), "user@icloud.com".into()]);
    assert!(ch.is_contact_allowed("+1234567890"));
    assert!(ch.is_contact_allowed("user@icloud.com"));
}

#[test]
fn unknown_contact_denied() {
    let ch = IMessageChannel::new(vec!["+1234567890".into()]);
    assert!(!ch.is_contact_allowed("+9999999999"));
    assert!(!ch.is_contact_allowed("hacker@evil.com"));
}

#[test]
fn contact_case_insensitive() {
    let ch = IMessageChannel::new(vec!["User@iCloud.com".into()]);
    assert!(ch.is_contact_allowed("user@icloud.com"));
    assert!(ch.is_contact_allowed("USER@ICLOUD.COM"));
}

#[test]
fn empty_allowlist_denies_all() {
    let ch = IMessageChannel::new(vec![]);
    assert!(!ch.is_contact_allowed("+1234567890"));
    assert!(!ch.is_contact_allowed("anyone"));
}

#[test]
fn name_returns_imessage() {
    let ch = IMessageChannel::new(vec![]);
    assert_eq!(ch.name(), "imessage");
}

#[test]
fn wildcard_among_others_still_allows_all() {
    let ch = IMessageChannel::new(vec!["+111".into(), "*".into(), "+222".into()]);
    assert!(ch.is_contact_allowed("totally-unknown"));
}

#[test]
fn contact_with_spaces_exact_match() {
    let ch = IMessageChannel::new(vec!["  spaced  ".into()]);
    assert!(ch.is_contact_allowed("  spaced  "));
    assert!(!ch.is_contact_allowed("spaced"));
}

// ══════════════════════════════════════════════════════════
// AppleScript Escaping Tests (CWE-78 Prevention)
// ══════════════════════════════════════════════════════════

#[test]
fn escape_applescript_double_quotes() {
    assert_eq!(escape_applescript(r#"hello "world""#), r#"hello \"world\""#);
}

#[test]
fn escape_applescript_backslashes() {
    assert_eq!(escape_applescript(r"path\to\file"), r"path\\to\\file");
}

#[test]
fn escape_applescript_mixed() {
    assert_eq!(
        escape_applescript(r#"say "hello\" world"#),
        r#"say \"hello\\\" world"#
    );
}

#[test]
fn escape_applescript_injection_attempt() {
    // This is the exact attack vector from the security report
    let malicious = r#"" & do shell script "id" & ""#;
    let escaped = escape_applescript(malicious);
    // After escaping, the quotes should be escaped and not break out
    assert_eq!(escaped, r#"\" & do shell script \"id\" & \""#);
    // Verify all quotes are now escaped (preceded by backslash)
    // The escaped string should not have any unescaped quotes (quote not preceded by backslash)
    let chars: Vec<char> = escaped.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c == '"' {
            // Every quote must be preceded by a backslash
            assert!(
                i > 0 && chars[i - 1] == '\\',
                "Found unescaped quote at position {i}"
            );
        }
    }
}

#[test]
fn escape_applescript_empty_string() {
    assert_eq!(escape_applescript(""), "");
}

#[test]
fn escape_applescript_no_special_chars() {
    assert_eq!(escape_applescript("hello world"), "hello world");
}

#[test]
fn escape_applescript_unicode() {
    assert_eq!(escape_applescript("hello 🦀 world"), "hello 🦀 world");
}

#[test]
fn escape_applescript_newlines_escaped() {
    assert_eq!(escape_applescript("line1\nline2"), "line1\\nline2");
    assert_eq!(escape_applescript("line1\rline2"), "line1\\rline2");
    assert_eq!(escape_applescript("line1\r\nline2"), "line1\\r\\nline2");
}

// ══════════════════════════════════════════════════════════
// Target Validation Tests
// ══════════════════════════════════════════════════════════

#[test]
fn valid_phone_number_simple() {
    assert!(is_valid_imessage_target("+1234567890"));
}

#[test]
fn valid_phone_number_with_country_code() {
    assert!(is_valid_imessage_target("+14155551234"));
}

#[test]
fn valid_phone_number_with_spaces() {
    assert!(is_valid_imessage_target("+1 415 555 1234"));
}

#[test]
fn valid_phone_number_with_dashes() {
    assert!(is_valid_imessage_target("+1-415-555-1234"));
}

#[test]
fn valid_phone_number_international() {
    assert!(is_valid_imessage_target("+447911123456")); // UK
    assert!(is_valid_imessage_target("+81312345678")); // Japan
}

#[test]
fn valid_email_simple() {
    assert!(is_valid_imessage_target("user@example.com"));
}

#[test]
fn valid_email_with_subdomain() {
    assert!(is_valid_imessage_target("user@mail.example.com"));
}

#[test]
fn valid_email_with_plus() {
    assert!(is_valid_imessage_target("user+tag@example.com"));
}

#[test]
fn valid_email_with_dots() {
    assert!(is_valid_imessage_target("first.last@example.com"));
}

#[test]
fn valid_email_icloud() {
    assert!(is_valid_imessage_target("user@icloud.com"));
    assert!(is_valid_imessage_target("user@me.com"));
}

#[test]
fn invalid_target_empty() {
    assert!(!is_valid_imessage_target(""));
    assert!(!is_valid_imessage_target("   "));
}

#[test]
fn invalid_target_no_plus_prefix() {
    // Phone numbers must start with +
    assert!(!is_valid_imessage_target("1234567890"));
}

#[test]
fn invalid_target_too_short_phone() {
    // Less than 7 digits
    assert!(!is_valid_imessage_target("+123456"));
}

#[test]
fn invalid_target_too_long_phone() {
    // More than 15 digits
    assert!(!is_valid_imessage_target("+1234567890123456"));
}

#[test]
fn invalid_target_email_no_at() {
    assert!(!is_valid_imessage_target("userexample.com"));
}

#[test]
fn invalid_target_email_no_domain() {
    assert!(!is_valid_imessage_target("user@"));
}

#[test]
fn invalid_target_email_no_local() {
    assert!(!is_valid_imessage_target("@example.com"));
}

#[test]
fn invalid_target_email_no_dot_in_domain() {
    assert!(!is_valid_imessage_target("user@localhost"));
}

#[test]
fn invalid_target_injection_attempt() {
    // The exact attack vector from the security report
    assert!(!is_valid_imessage_target(r#"" & do shell script "id" & ""#));
}

#[test]
fn invalid_target_applescript_injection() {
    // Various injection attempts
    assert!(!is_valid_imessage_target(r#"test" & quit"#));
    assert!(!is_valid_imessage_target(r"test\ndo shell script"));
    assert!(!is_valid_imessage_target("test\"; malicious code; \""));
}

#[test]
fn invalid_target_special_chars() {
    assert!(!is_valid_imessage_target("user<script>@example.com"));
    assert!(!is_valid_imessage_target("user@example.com; rm -rf /"));
}

#[test]
fn invalid_target_null_byte() {
    assert!(!is_valid_imessage_target("user\0@example.com"));
}

#[test]
fn invalid_target_newline() {
    assert!(!is_valid_imessage_target("user\n@example.com"));
}

#[test]
fn target_with_leading_trailing_whitespace_trimmed() {
    // Should trim and validate
    assert!(is_valid_imessage_target("  +1234567890  "));
    assert!(is_valid_imessage_target("  user@example.com  "));
}

// ══════════════════════════════════════════════════════════
// SQLite/rusqlite Database Tests (CWE-89 Prevention)
// ══════════════════════════════════════════════════════════

/// Helper to create a temporary test database with Messages schema
fn create_test_db() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("chat.db");

    let conn = Connection::open(&db_path).unwrap();

    // Create minimal schema matching macOS Messages.app
    conn.execute_batch(
        "CREATE TABLE handle (
            ROWID INTEGER PRIMARY KEY,
            id TEXT NOT NULL
        );
        CREATE TABLE message (
            ROWID INTEGER PRIMARY KEY,
            handle_id INTEGER,
            text TEXT,
            is_from_me INTEGER DEFAULT 0,
            FOREIGN KEY (handle_id) REFERENCES handle(ROWID)
        );",
    )
    .unwrap();

    (dir, db_path)
}

#[tokio::test]
async fn get_max_rowid_empty_database() {
    let (_dir, db_path) = create_test_db();
    let result = get_max_rowid(&db_path).await;
    assert!(result.is_ok());
    // Empty table returns 0 (NULL coalesced)
    assert_eq!(result.unwrap(), 0);
}

#[tokio::test]
async fn get_max_rowid_with_messages() {
    let (_dir, db_path) = create_test_db();

    // Insert test data
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO handle (ROWID, id) VALUES (1, '+1234567890')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (100, 1, 'Hello', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (200, 1, 'World', 0)",
            [],
        )
        .unwrap();
        // This one is from_me=1, should be ignored
        conn.execute(
            "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (300, 1, 'Sent', 1)",
            [],
        )
        .unwrap();
    }

    let result = get_max_rowid(&db_path).await.unwrap();
    // Should return 200, not 300 (ignores is_from_me=1)
    assert_eq!(result, 200);
}

#[tokio::test]
async fn get_max_rowid_nonexistent_database() {
    let path = std::path::Path::new("/nonexistent/path/chat.db");
    let result = get_max_rowid(path).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn fetch_new_messages_empty_database() {
    let (_dir, db_path) = create_test_db();
    let result = fetch_new_messages(&db_path, 0).await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}

#[tokio::test]
async fn fetch_new_messages_returns_correct_data() {
    let (_dir, db_path) = create_test_db();

    // Insert test data
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO handle (ROWID, id) VALUES (1, '+1234567890')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO handle (ROWID, id) VALUES (2, 'user@example.com')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (10, 1, 'First message', 0)",
            []
        ).unwrap();
        conn.execute(
            "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (20, 2, 'Second message', 0)",
            []
        ).unwrap();
    }

    let result = fetch_new_messages(&db_path, 0).await.unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(
        result[0],
        (10, "+1234567890".to_string(), "First message".to_string())
    );
    assert_eq!(
        result[1],
        (
            20,
            "user@example.com".to_string(),
            "Second message".to_string()
        )
    );
}

#[tokio::test]
async fn fetch_new_messages_filters_by_rowid() {
    let (_dir, db_path) = create_test_db();

    // Insert test data
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO handle (ROWID, id) VALUES (1, '+1234567890')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (10, 1, 'Old message', 0)",
            []
        ).unwrap();
        conn.execute(
            "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (20, 1, 'New message', 0)",
            []
        ).unwrap();
    }

    // Fetch only messages after ROWID 15
    let result = fetch_new_messages(&db_path, 15).await.unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, 20);
    assert_eq!(result[0].2, "New message");
}

#[tokio::test]
async fn fetch_new_messages_excludes_sent_messages() {
    let (_dir, db_path) = create_test_db();

    // Insert test data
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO handle (ROWID, id) VALUES (1, '+1234567890')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (10, 1, 'Received', 0)",
            []
        ).unwrap();
        conn.execute(
            "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (20, 1, 'Sent by me', 1)",
            []
        ).unwrap();
    }

    let result = fetch_new_messages(&db_path, 0).await.unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].2, "Received");
}

#[tokio::test]
async fn fetch_new_messages_excludes_null_text() {
    let (_dir, db_path) = create_test_db();

    // Insert test data
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO handle (ROWID, id) VALUES (1, '+1234567890')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (10, 1, 'Has text', 0)",
            []
        ).unwrap();
        conn.execute(
            "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (20, 1, NULL, 0)",
            [],
        )
        .unwrap();
    }

    let result = fetch_new_messages(&db_path, 0).await.unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].2, "Has text");
}

#[tokio::test]
async fn fetch_new_messages_respects_limit() {
    let (_dir, db_path) = create_test_db();

    // Insert 25 messages (limit is 20)
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO handle (ROWID, id) VALUES (1, '+1234567890')",
            [],
        )
        .unwrap();
        for i in 1..=25 {
            conn.execute(
                &format!("INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES ({i}, 1, 'Message {i}', 0)"),
                []
            ).unwrap();
        }
    }

    let result = fetch_new_messages(&db_path, 0).await.unwrap();
    assert_eq!(result.len(), 20); // Limited to 20
    assert_eq!(result[0].0, 1); // First message
    assert_eq!(result[19].0, 20); // 20th message
}

#[tokio::test]
async fn fetch_new_messages_ordered_by_rowid_asc() {
    let (_dir, db_path) = create_test_db();

    // Insert messages out of order
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO handle (ROWID, id) VALUES (1, '+1234567890')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (30, 1, 'Third', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (10, 1, 'First', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (20, 1, 'Second', 0)",
            [],
        )
        .unwrap();
    }

    let result = fetch_new_messages(&db_path, 0).await.unwrap();
    assert_eq!(result.len(), 3);
    assert_eq!(result[0].0, 10);
    assert_eq!(result[1].0, 20);
    assert_eq!(result[2].0, 30);
}

#[tokio::test]
async fn fetch_new_messages_nonexistent_database() {
    let path = std::path::Path::new("/nonexistent/path/chat.db");
    let result = fetch_new_messages(path, 0).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn fetch_new_messages_handles_special_characters() {
    let (_dir, db_path) = create_test_db();

    // Insert message with special characters (potential SQL injection patterns)
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO handle (ROWID, id) VALUES (1, '+1234567890')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (10, 1, 'Hello \"world'' OR 1=1; DROP TABLE message;--', 0)",
            []
        ).unwrap();
    }

    let result = fetch_new_messages(&db_path, 0).await.unwrap();
    assert_eq!(result.len(), 1);
    // The special characters should be preserved, not interpreted as SQL
    assert!(result[0].2.contains("DROP TABLE"));
}

#[tokio::test]
async fn fetch_new_messages_handles_unicode() {
    let (_dir, db_path) = create_test_db();

    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO handle (ROWID, id) VALUES (1, '+1234567890')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (10, 1, 'Hello 🦀 世界 مرحبا', 0)",
            []
        ).unwrap();
    }

    let result = fetch_new_messages(&db_path, 0).await.unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].2, "Hello 🦀 世界 مرحبا");
}

#[tokio::test]
async fn fetch_new_messages_handles_empty_text() {
    let (_dir, db_path) = create_test_db();

    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO handle (ROWID, id) VALUES (1, '+1234567890')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (10, 1, '', 0)",
            [],
        )
        .unwrap();
    }

    let result = fetch_new_messages(&db_path, 0).await.unwrap();
    // Empty string is NOT NULL, so it's included
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].2, "");
}

#[tokio::test]
async fn fetch_new_messages_negative_rowid_edge_case() {
    let (_dir, db_path) = create_test_db();

    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO handle (ROWID, id) VALUES (1, '+1234567890')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (10, 1, 'Test', 0)",
            [],
        )
        .unwrap();
    }

    // Negative rowid should still work (fetch all messages with ROWID > -1)
    let result = fetch_new_messages(&db_path, -1).await.unwrap();
    assert_eq!(result.len(), 1);
}

#[tokio::test]
async fn fetch_new_messages_large_rowid_edge_case() {
    let (_dir, db_path) = create_test_db();

    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO handle (ROWID, id) VALUES (1, '+1234567890')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (10, 1, 'Test', 0)",
            [],
        )
        .unwrap();
    }

    // Very large rowid should return empty (no messages after this)
    let result = fetch_new_messages(&db_path, i64::MAX - 1).await.unwrap();
    assert!(result.is_empty());
}

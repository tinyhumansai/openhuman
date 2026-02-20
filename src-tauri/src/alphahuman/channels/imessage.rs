use crate::alphahuman::channels::traits::{Channel, ChannelMessage, SendMessage};
use async_trait::async_trait;
use directories::UserDirs;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use tokio::sync::mpsc;

/// iMessage channel using macOS `AppleScript` bridge.
/// Polls the Messages database for new messages and sends replies via `osascript`.
#[derive(Clone)]
pub struct IMessageChannel {
    allowed_contacts: Vec<String>,
    poll_interval_secs: u64,
}

impl IMessageChannel {
    pub fn new(allowed_contacts: Vec<String>) -> Self {
        Self {
            allowed_contacts,
            poll_interval_secs: 3,
        }
    }

    fn is_contact_allowed(&self, sender: &str) -> bool {
        if self.allowed_contacts.iter().any(|u| u == "*") {
            return true;
        }
        self.allowed_contacts
            .iter()
            .any(|u| u.eq_ignore_ascii_case(sender))
    }
}

/// Escape a string for safe interpolation into `AppleScript`.
///
/// This prevents injection attacks by escaping:
/// - Backslashes (`\` → `\\`)
/// - Double quotes (`"` → `\"`)
/// - Newlines (`\n` → `\\n`, `\r` → `\\r`) to prevent code injection via line breaks
fn escape_applescript(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// Validate that a target looks like a valid phone number or email address.
///
/// This is a defense-in-depth measure to reject obviously malicious targets
/// before they reach `AppleScript` interpolation.
///
/// Valid patterns:
/// - Phone: starts with `+` followed by digits (with optional spaces/dashes)
/// - Email: contains `@` with alphanumeric chars on both sides
fn is_valid_imessage_target(target: &str) -> bool {
    let target = target.trim();
    if target.is_empty() {
        return false;
    }

    // Phone number: +1234567890 or +1 234-567-8900
    if target.starts_with('+') {
        let digits_only: String = target.chars().filter(char::is_ascii_digit).collect();
        // Must have at least 7 digits (shortest valid phone numbers)
        return digits_only.len() >= 7 && digits_only.len() <= 15;
    }

    // Email: simple validation (contains @ with chars on both sides)
    if let Some(at_pos) = target.find('@') {
        let local = &target[..at_pos];
        let domain = &target[at_pos + 1..];

        // Local part: non-empty, alphanumeric + common email chars
        let local_valid = !local.is_empty()
            && local
                .chars()
                .all(|c| c.is_alphanumeric() || "._+-".contains(c));

        // Domain: non-empty, contains a dot, alphanumeric + dots/hyphens
        let domain_valid = !domain.is_empty()
            && domain.contains('.')
            && domain
                .chars()
                .all(|c| c.is_alphanumeric() || ".-".contains(c));

        return local_valid && domain_valid;
    }

    false
}

#[async_trait]
impl Channel for IMessageChannel {
    fn name(&self) -> &str {
        "imessage"
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        // Defense-in-depth: validate target format before any interpolation
        if !is_valid_imessage_target(&message.recipient) {
            anyhow::bail!(
                "Invalid iMessage target: must be a phone number (+1234567890) or email (user@example.com)"
            );
        }

        // SECURITY: Escape both message AND target to prevent AppleScript injection
        // See: CWE-78 (OS Command Injection)
        let escaped_msg = escape_applescript(&message.content);
        let escaped_target = escape_applescript(&message.recipient);

        let script = format!(
            r#"tell application "Messages"
    set targetService to 1st account whose service type = iMessage
    set targetBuddy to participant "{escaped_target}" of targetService
    send "{escaped_msg}" to targetBuddy
end tell"#
        );

        let output = tokio::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("iMessage send failed: {stderr}");
        }

        Ok(())
    }

    async fn listen(&self, tx: mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        tracing::info!("iMessage channel listening (AppleScript bridge)...");

        // Query the Messages SQLite database for new messages
        // The database is at ~/Library/Messages/chat.db
        let db_path = UserDirs::new()
            .map(|u| u.home_dir().join("Library/Messages/chat.db"))
            .ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?;

        if !db_path.exists() {
            anyhow::bail!(
                "Messages database not found at {}. Ensure Messages.app is set up and Full Disk Access is granted.",
                db_path.display()
            );
        }

        // Open a persistent read-only connection instead of creating
        // a new one on every 3-second poll cycle.
        let path = db_path.to_path_buf();
        let conn = tokio::task::spawn_blocking(move || -> anyhow::Result<Connection> {
            Ok(Connection::open_with_flags(
                &path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?)
        })
        .await??;

        // Track the last ROWID we've seen (shuttle conn in and out)
        let (mut conn, initial_rowid) =
            tokio::task::spawn_blocking(move || -> anyhow::Result<(Connection, i64)> {
                let rowid = {
                    let mut stmt =
                        conn.prepare("SELECT MAX(ROWID) FROM message WHERE is_from_me = 0")?;
                    let rowid: Option<i64> = stmt.query_row([], |row| row.get(0))?;
                    rowid.unwrap_or(0)
                };
                Ok((conn, rowid))
            })
            .await??;
        let mut last_rowid = initial_rowid;

        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(self.poll_interval_secs)).await;

            let since = last_rowid;
            let (returned_conn, poll_result) = tokio::task::spawn_blocking(
                move || -> (Connection, anyhow::Result<Vec<(i64, String, String)>>) {
                    let result = (|| -> anyhow::Result<Vec<(i64, String, String)>> {
                        let mut stmt = conn.prepare(
                            "SELECT m.ROWID, h.id, m.text \
                     FROM message m \
                     JOIN handle h ON m.handle_id = h.ROWID \
                     WHERE m.ROWID > ?1 \
                     AND m.is_from_me = 0 \
                     AND m.text IS NOT NULL \
                     ORDER BY m.ROWID ASC \
                     LIMIT 20",
                        )?;
                        let rows = stmt.query_map([since], |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                            ))
                        })?;
                        let results = rows.collect::<Result<Vec<_>, _>>()?;
                        Ok(results)
                    })();

                    (conn, result)
                },
            )
            .await
            .map_err(|e| anyhow::anyhow!("iMessage poll worker join error: {e}"))?;
            conn = returned_conn;

            match poll_result {
                Ok(messages) => {
                    for (rowid, sender, text) in messages {
                        if rowid > last_rowid {
                            last_rowid = rowid;
                        }

                        if !self.is_contact_allowed(&sender) {
                            continue;
                        }

                        if text.trim().is_empty() {
                            continue;
                        }

                        let msg = ChannelMessage {
                            id: rowid.to_string(),
                            sender: sender.clone(),
                            reply_target: sender.clone(),
                            content: text,
                            channel: "imessage".to_string(),
                            timestamp: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs(),
                            thread_ts: None,
                        };

                        if tx.send(msg).await.is_err() {
                            return Ok(());
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("iMessage poll error: {e}");
                }
            }
        }
    }

    async fn health_check(&self) -> bool {
        if !cfg!(target_os = "macos") {
            return false;
        }

        let db_path = UserDirs::new()
            .map(|u| u.home_dir().join("Library/Messages/chat.db"))
            .unwrap_or_default();

        db_path.exists()
    }
}

/// Get the current max ROWID from the messages table.
/// Uses rusqlite with parameterized queries for security (CWE-89 prevention).
async fn get_max_rowid(db_path: &Path) -> anyhow::Result<i64> {
    let path = db_path.to_path_buf();
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<i64> {
        let conn = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let mut stmt = conn.prepare("SELECT MAX(ROWID) FROM message WHERE is_from_me = 0")?;
        let rowid: Option<i64> = stmt.query_row([], |row| row.get(0))?;
        Ok(rowid.unwrap_or(0))
    })
    .await??;
    Ok(result)
}

/// Fetch messages newer than `since_rowid`.
/// Uses rusqlite with parameterized queries for security (CWE-89 prevention).
/// The `since_rowid` parameter is bound safely, preventing SQL injection.
async fn fetch_new_messages(
    db_path: &Path,
    since_rowid: i64,
) -> anyhow::Result<Vec<(i64, String, String)>> {
    let path = db_path.to_path_buf();
    let results =
        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<(i64, String, String)>> {
            let conn = Connection::open_with_flags(
                &path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            let mut stmt = conn.prepare(
                "SELECT m.ROWID, h.id, m.text \
             FROM message m \
             JOIN handle h ON m.handle_id = h.ROWID \
             WHERE m.ROWID > ?1 \
             AND m.is_from_me = 0 \
             AND m.text IS NOT NULL \
             ORDER BY m.ROWID ASC \
             LIMIT 20",
            )?;
            let rows = stmt.query_map([since_rowid], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
        .await??;
    Ok(results)
}

#[cfg(test)]
mod tests {
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
                []
            ).unwrap();
            conn.execute(
                "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (200, 1, 'World', 0)",
                []
            ).unwrap();
            // This one is from_me=1, should be ignored
            conn.execute(
                "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (300, 1, 'Sent', 1)",
                []
            ).unwrap();
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
                []
            ).unwrap();
            conn.execute(
                "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (10, 1, 'First', 0)",
                []
            ).unwrap();
            conn.execute(
                "INSERT INTO message (ROWID, handle_id, text, is_from_me) VALUES (20, 1, 'Second', 0)",
                []
            ).unwrap();
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
                []
            ).unwrap();
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
                []
            ).unwrap();
        }

        // Very large rowid should return empty (no messages after this)
        let result = fetch_new_messages(&db_path, i64::MAX - 1).await.unwrap();
        assert!(result.is_empty());
    }
}

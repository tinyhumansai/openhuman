use crate::openhuman::channels::traits::{Channel, ChannelMessage, SendMessage};
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
        if std::env::consts::OS != "macos" {
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
#[path = "imessage_tests.rs"]
mod tests;

//! Public types for the native Gmail integration domain.

use serde::{Deserialize, Serialize};

/// A connected Gmail account tracked by the domain store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmailAccount {
    /// Opaque stable identifier chosen by the caller at connect time.
    /// Typically the same value used as the webview `account_id`.
    pub account_id: String,
    /// The Google account email address as discovered from the page DOM or
    /// from user input at connect time.
    pub email: String,
    /// Unix-ms timestamp when the account was connected.
    pub connected_at_ms: i64,
    /// Unix-ms timestamp of the most recent completed sync, or 0 if the
    /// account has never been synced.
    pub last_sync_at_ms: i64,
    /// Number of messages ingested in the last sync pass.
    pub last_sync_count: i64,
    /// The cron job id responsible for periodic re-sync, if one was
    /// registered at connect time.
    pub cron_job_id: Option<String>,
}

/// A normalised Gmail message ready for ingestion into the memory layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmailMessage {
    /// Stable Gmail message id (e.g. `17abc1234def5678`).
    pub id: String,
    /// Gmail thread id.
    pub thread_id: String,
    /// `From:` header value.
    pub from: String,
    /// `To:` header value (may be a comma-separated list).
    pub to: String,
    /// Decoded `Subject:` header.
    pub subject: String,
    /// Short preview / snippet (≤ 200 chars).
    pub snippet: String,
    /// Plain-text body (preferred) or HTML body when no text part is present.
    pub body: String,
    /// Gmail label ids (e.g. `INBOX`, `SENT`, `UNREAD`).
    pub labels: Vec<String>,
    /// Message timestamp in milliseconds since epoch.
    pub ts_ms: i64,
}

impl GmailMessage {
    /// Whether the message carries the `UNREAD` label.
    pub fn is_unread(&self) -> bool {
        self.labels.iter().any(|l| l.eq_ignore_ascii_case("UNREAD"))
    }

    /// Primary category label for memory `category` field.
    /// Returns the first "well-known" label or `"other"`.
    pub fn primary_category(&self) -> &str {
        for label in &self.labels {
            match label.to_uppercase().as_str() {
                "INBOX" => return "inbox",
                "SENT" => return "sent",
                "DRAFT" => return "draft",
                "SPAM" => return "spam",
                "TRASH" => return "trash",
                _ => {}
            }
        }
        "other"
    }
}

/// Summary statistics returned by `gmail.get_stats`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmailSyncStats {
    pub account_id: String,
    pub email: String,
    pub connected_at_ms: i64,
    pub last_sync_at_ms: i64,
    pub last_sync_count: i64,
    pub cron_job_id: Option<String>,
}

impl From<&GmailAccount> for GmailSyncStats {
    fn from(a: &GmailAccount) -> Self {
        Self {
            account_id: a.account_id.clone(),
            email: a.email.clone(),
            connected_at_ms: a.connected_at_ms,
            last_sync_at_ms: a.last_sync_at_ms,
            last_sync_count: a.last_sync_count,
            cron_job_id: a.cron_job_id.clone(),
        }
    }
}

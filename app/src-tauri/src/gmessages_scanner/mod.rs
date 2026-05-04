//! Google Messages Web scanner — Windows-focused, read-only IndexedDB walk.
//!
//! Scope for Stage 1:
//!   * Read-only scan of `bugle_db` (the IndexedDB database used by
//!     `messages.google.com/web`) via CDP on the embedded CEF webview.
//!   * One ingest call per `(thread_id, day)` group — same
//!     `openhuman.memory_doc_ingest` shape the iMessage and WhatsApp
//!     scanners already use.
//!   * No DOM automation. No send path. Send is deferred to a separate
//!     PR that will use OS Accessibility APIs (macOS AX / Windows UIA) —
//!     indistinguishable from a screen reader, so ToS-clean.
//!
//! Targeted at Windows + Android (the only practical combo for Google
//! Messages — iPhone owners use iMessage, mac users typically use
//! Messages Web in a browser tab that the CEF shell doesn't own). The
//! code is windows-gated at module-scope; on other targets the public
//! surface compiles to no-op stubs so `lib.rs` stays clean.
//!
//! History model differs from iMessage:
//!   * iMessage (#724) reads `chat.db` which holds FULL history locally.
//!   * Google Messages Web only caches in `bugle_db` what the web client
//!     has already synced. If the user never scrolled to older
//!     conversations, those pages aren't in IDB. Document this behavior
//!     in the UI — "scroll to backfill older history."
//!
//! CDP wiring TODO:
//!   * The CEF remote-debugging port + per-account target selection lives
//!     in `whatsapp_scanner::mod` today (`CDP_HOST`/`CDP_PORT`,
//!     `Target.getTargets` filter). When this module is promoted from
//!     scaffold to running scanner, lift that plumbing into a shared
//!     `cdp` module and point this scanner at the Google Messages Web
//!     target (`messages.google.com/web`). Until then `run_scanner` is a
//!     stub that logs and exits — the PR ships the normalization +
//!     memory-doc shape so downstream can iterate without the full CDP
//!     loop landed.

// Scaffold PR — orchestrator loop is a stub pending the shared CDP lift
// from `whatsapp_scanner`. Once that lands and this module actually
// drives `idb::walk` + `memory_doc_ingest`, drop the blanket allow below.
#![allow(dead_code)]

#[cfg(target_os = "windows")]
use std::sync::Arc;
#[cfg(target_os = "windows")]
use std::time::Duration;

#[cfg(target_os = "windows")]
use parking_lot::Mutex;
#[cfg(target_os = "windows")]
use tauri::{AppHandle, Runtime};

pub mod idb;

#[cfg(target_os = "windows")]
const SCAN_INTERVAL: Duration = Duration::from_secs(60);

/// Per-account scanner registry. Google Messages Web supports one paired
/// phone per browser session; the registry shape is kept symmetric with
/// the iMessage / WhatsApp scanners for future multi-account expansion.
#[cfg(target_os = "windows")]
pub struct ScannerRegistry {
    inner: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

#[cfg(target_os = "windows")]
impl ScannerRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    pub fn ensure_scanner<R: Runtime>(self: Arc<Self>, app: AppHandle<R>, account_id: String) {
        let mut guard = self.inner.lock();
        if guard.as_ref().map_or(false, |h| !h.is_finished()) {
            return;
        }
        let handle = tokio::spawn(run_scanner(app, account_id));
        *guard = Some(handle);
    }
}

/// Stub loop — logs and exits. Wire CDP target discovery + `idb::walk`
/// here once the shared `cdp` module is lifted from `whatsapp_scanner`.
/// See module-level TODO.
#[cfg(target_os = "windows")]
async fn run_scanner<R: Runtime>(_app: AppHandle<R>, account_id: String) {
    log::info!(
        "[gmessages] scanner scaffold loaded account={} interval={:?} — CDP wiring pending",
        account_id,
        SCAN_INTERVAL
    );
}

// Non-Windows stub so the rest of the app compiles unchanged on mac/linux.
#[cfg(not(target_os = "windows"))]
pub struct ScannerRegistry;

#[cfg(not(target_os = "windows"))]
impl ScannerRegistry {
    pub fn new() -> Self {
        Self
    }
    pub fn ensure_scanner<R: tauri::Runtime>(
        self: std::sync::Arc<Self>,
        _app: tauri::AppHandle<R>,
        _account_id: String,
    ) {
    }
}

/// Format a list of normalized messages into a transcript string suitable
/// for `memory_doc_ingest.content`. Matches the iMessage scanner output
/// shape so Neocortex sees a uniform format across channels.
pub fn format_transcript(messages: &[idb::Message], participants: &idb::ParticipantMap) -> String {
    let mut out = String::new();
    for m in messages {
        let sender = if m.from_me {
            "me".to_string()
        } else {
            m.sender_id
                .as_deref()
                .and_then(|sid| participants.display_name(sid))
                .unwrap_or_else(|| m.sender_id.clone().unwrap_or_else(|| "unknown".into()))
        };
        let text = m.text.replace('\n', " ");
        let body = if text.is_empty() {
            "[non-text]".to_string()
        } else {
            text
        };
        out.push_str(&format!("[{}] {}: {}\n", m.timestamp_unix, sender, body));
    }
    out
}

/// Group a flat list of messages into `(thread_id, YYYY-MM-DD) -> Vec<Message>`.
/// Day bucketing uses the local timezone — users inspect memory docs by
/// their calendar day, not UTC (same policy as iMessage #724 after the
/// CodeRabbit local-TZ fix).
pub fn group_by_thread_day(
    messages: Vec<idb::Message>,
) -> Vec<((String, String), Vec<idb::Message>)> {
    use std::collections::BTreeMap;
    let mut groups: BTreeMap<(String, String), Vec<idb::Message>> = BTreeMap::new();
    for m in messages {
        let Some(thread_id) = m.thread_id.clone() else {
            continue;
        };
        let day = seconds_to_ymd(m.timestamp_unix);
        groups.entry((thread_id, day)).or_default().push(m);
    }
    groups.into_iter().collect()
}

/// Local-timezone day bucket for a unix-second timestamp. Returns
/// "YYYY-MM-DD" or "unknown" for values that fall outside chrono's range.
pub fn seconds_to_ymd(secs: i64) -> String {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_opt(secs, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str, thread: &str, ts: i64, text: &str, from_me: bool) -> idb::Message {
        idb::Message {
            id: id.into(),
            thread_id: Some(thread.into()),
            sender_id: if from_me {
                None
            } else {
                Some("+15551234567".into())
            },
            from_me,
            text: text.into(),
            timestamp_unix: ts,
            message_type: Some("sms".into()),
        }
    }

    #[test]
    fn group_by_thread_day_buckets_messages_correctly() {
        // Two messages ~5s apart in the same thread should fall into one
        // group; a third in a different thread into its own group.
        let base = 1_700_000_000;
        let msgs = vec![
            msg("1", "t1", base, "hi", false),
            msg("2", "t1", base + 5, "yo", true),
            msg("3", "t2", base, "other", false),
        ];
        let groups = group_by_thread_day(msgs);
        assert_eq!(groups.len(), 2);
        let t1 = groups.iter().find(|((t, _), _)| t == "t1").unwrap();
        assert_eq!(t1.1.len(), 2);
    }

    #[test]
    fn format_transcript_includes_sender_and_body() {
        let msgs = vec![
            msg("1", "t1", 1_700_000_000, "hi", false),
            msg("2", "t1", 1_700_000_005, "yo", true),
        ];
        let participants = idb::ParticipantMap::default();
        let t = format_transcript(&msgs, &participants);
        assert!(t.contains("hi"));
        assert!(t.contains("me: yo"));
        assert!(t.contains("+15551234567: hi"));
    }

    #[test]
    fn format_transcript_resolves_display_name_from_participants() {
        let mut participants = idb::ParticipantMap::default();
        participants.insert("+15551234567".into(), "Alice".into());
        let msgs = vec![msg("1", "t1", 1_700_000_000, "hi", false)];
        let t = format_transcript(&msgs, &participants);
        assert!(t.contains("Alice: hi"), "got {:?}", t);
    }

    #[test]
    fn format_transcript_marks_empty_body_as_non_text() {
        let msgs = vec![msg("1", "t1", 1_700_000_000, "", false)];
        let t = format_transcript(&msgs, &idb::ParticipantMap::default());
        assert!(t.contains("[non-text]"), "got {:?}", t);
    }

    #[test]
    fn seconds_to_ymd_shape() {
        let out = seconds_to_ymd(1_700_000_000);
        assert_eq!(out.len(), 10);
        assert_eq!(&out[4..5], "-");
        assert_eq!(&out[7..8], "-");
    }
}

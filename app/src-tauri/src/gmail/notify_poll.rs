//! Gmail Atom-feed polling task that surfaces native OS notifications
//! for newly-arriving unread mail.
//!
//! Why polling rather than relying on the page: Gmail's BrowserChannel
//! real-time push (`mail.google.com/mail/u/0/channel/bind`) does not
//! deliver to CEF webviews, and Web Push (FCM service worker) requires
//! Chromium FCM glue absent in CEF builds. As a result the page's own
//! `new Notification(...)` calls never fire, even with the
//! `Browser.grantPermissions(["notifications"])` and JS shim that
//! `cdp::session` installs. We bypass the page entirely by polling the
//! stable Atom feed and dispatching synthetic toasts via
//! `forward_synthetic_notification` for unread IDs we have not seen
//! before.
//!
//! Lifecycle: one task per opened gmail webview account, started at
//! `webview_account_open` and aborted at `webview_account_close` /
//! `webview_account_purge`. The `JoinHandle` is kept in
//! `WebviewAccountsState.gmail_notify_polls` keyed by account id.
//!
//! Persistence: the seen-id set is mirrored to
//! `<workspace>/gmail/<account_id>/seen.json` so a relaunch does not
//! replay the most-recent unread backlog as toasts. The set is capped
//! at [`SEEN_SET_CAP`] entries with FIFO eviction; this is roughly six
//! times the Gmail Atom-feed window of 20 messages so IDs that briefly
//! rotate out and back in are not re-fired.
//!
//! All gating (DnD, mute, focused-account bypass, the global
//! `NotificationSettings` toggle) is inherited via
//! `forward_synthetic_notification` → `forward_native_notification`.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};
use tokio::task::JoinHandle;
use tokio::time::{interval, MissedTickBehavior};

use crate::gmail;
use crate::webview_accounts::forward_synthetic_notification;

/// Time between successive Atom-feed polls.
pub const POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Initial delay before the first poll. Gives the per-account CDP
/// session time to attach and Gmail's first paint to settle so the
/// authenticated `cdp_fetch` call does not race the navigation.
pub const INITIAL_DELAY: Duration = Duration::from_secs(15);

/// How many unread IDs to request from the Atom feed per poll. Gmail
/// caps the feed at 20 most-recent unread regardless of larger limits,
/// so this is the natural ceiling.
pub const POLL_LIMIT: u32 = 20;

/// Maximum number of IDs retained in the persistent seen-set. Older
/// entries are evicted FIFO on overflow. Sized to ~6× the Atom-feed
/// window so IDs that briefly leave and re-enter the window are not
/// re-fired.
pub const SEEN_SET_CAP: usize = 200;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct SeenStateOnDisk {
    /// Insertion-ordered list of message IDs we have already toasted on
    /// (or seeded from the first poll). Front is oldest, back is newest.
    ids: Vec<String>,
}

/// In-memory mirror of the persistent seen-set. `set` is the lookup
/// index, `order` preserves insertion order for FIFO eviction; both
/// hold the same set of strings.
#[derive(Debug, Default)]
struct SeenState {
    set: HashSet<String>,
    order: VecDeque<String>,
}

impl SeenState {
    fn from_disk(state: SeenStateOnDisk) -> Self {
        let mut out = Self::default();
        for id in state.ids {
            out.insert(id);
        }
        out
    }

    fn to_disk(&self) -> SeenStateOnDisk {
        SeenStateOnDisk {
            ids: self.order.iter().cloned().collect(),
        }
    }

    /// Insert `id` into the seen-set. Returns `true` when the id is
    /// new, `false` when it was already present (callers use this to
    /// decide whether to fire). Evicts the oldest entry when the set
    /// would exceed [`SEEN_SET_CAP`].
    fn insert(&mut self, id: String) -> bool {
        if self.set.contains(&id) {
            return false;
        }
        self.set.insert(id.clone());
        self.order.push_back(id);
        while self.order.len() > SEEN_SET_CAP {
            if let Some(evicted) = self.order.pop_front() {
                self.set.remove(&evicted);
            }
        }
        true
    }
}

/// Build the path to the seen-set JSON file for a given account under
/// the OpenHuman workspace dir.
fn seen_set_path(workspace_dir: &Path, account_id: &str) -> PathBuf {
    workspace_dir
        .join("gmail")
        .join(account_id)
        .join("seen.json")
}

fn load_seen(path: &Path) -> SeenState {
    match std::fs::read_to_string(path) {
        Ok(raw) => match serde_json::from_str::<SeenStateOnDisk>(&raw) {
            Ok(state) => SeenState::from_disk(state),
            Err(e) => {
                log::warn!(
                    "[gmail-notify-poll] could not parse {}: {} — starting empty",
                    path.display(),
                    e
                );
                SeenState::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => SeenState::default(),
        Err(e) => {
            log::warn!(
                "[gmail-notify-poll] could not read {}: {} — starting empty",
                path.display(),
                e
            );
            SeenState::default()
        }
    }
}

fn save_seen(path: &Path, state: &SeenState) {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            log::warn!(
                "[gmail-notify-poll] mkdir {} failed: {}",
                parent.display(),
                e
            );
            return;
        }
    }
    let body = match serde_json::to_string(&state.to_disk()) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("[gmail-notify-poll] serialize seen-set failed: {}", e);
            return;
        }
    };
    if let Err(e) = std::fs::write(path, body) {
        log::warn!("[gmail-notify-poll] write {} failed: {}", path.display(), e);
    }
}

/// Spawn the per-account Gmail notification poll task. The returned
/// `JoinHandle` should be stored in `WebviewAccountsState.gmail_notify_polls`
/// so `webview_account_close` and `webview_account_purge` can abort it.
pub fn spawn<R: Runtime>(
    app: AppHandle<R>,
    account_id: String,
    workspace_dir: PathBuf,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let path = seen_set_path(&workspace_dir, &account_id);
        let mut seen = load_seen(&path);
        let resumed = !seen.set.is_empty();
        log::info!(
            "[gmail-notify-poll][{}] starting (interval={}s, limit={}, seen_loaded={}, path={})",
            account_id,
            POLL_INTERVAL.as_secs(),
            POLL_LIMIT,
            seen.set.len(),
            path.display()
        );

        // Wait for the CDP session to attach and Gmail to finish its
        // initial load before the first fetch.
        tokio::time::sleep(INITIAL_DELAY).await;

        let mut tick = interval(POLL_INTERVAL);
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        // Whether the first poll of this task instance has run yet. A
        // brand-new task with no persisted state seeds silently on its
        // first poll; a resumed task already has IDs on disk and treats
        // every new ID as fireable from tick 1.
        let mut seeded = resumed;

        loop {
            tick.tick().await;
            let result = gmail::cdp_list_messages_uncached(&account_id, POLL_LIMIT, None).await;
            match result {
                Ok(msgs) => {
                    log::info!(
                        "[gmail-notify-poll][{}] fetched={} seeded={} seen_pre={}",
                        account_id,
                        msgs.len(),
                        seeded,
                        seen.set.len()
                    );
                    let mut wrote_any = false;
                    for m in &msgs {
                        if m.id.is_empty() {
                            continue;
                        }
                        let is_new = seen.insert(m.id.clone());
                        if is_new {
                            wrote_any = true;
                            if seeded {
                                fire_toast(&app, &account_id, m);
                            }
                        }
                    }
                    if wrote_any {
                        save_seen(&path, &seen);
                    }
                    seeded = true;
                }
                Err(e) => {
                    log::warn!("[gmail-notify-poll][{}] fetch failed: {}", account_id, e);
                }
            }
        }
    })
}

fn fire_toast<R: Runtime>(app: &AppHandle<R>, account_id: &str, msg: &gmail::types::GmailMessage) {
    let title = msg
        .from
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Gmail".to_string());
    let body = match (msg.subject.as_deref(), msg.snippet.as_deref()) {
        (Some(s), Some(sn)) if !s.is_empty() && !sn.is_empty() => format!("{s}\n{sn}"),
        (Some(s), _) if !s.is_empty() => s.to_string(),
        (_, Some(sn)) if !sn.is_empty() => sn.to_string(),
        _ => "New message".to_string(),
    };
    log::info!(
        "[gmail-notify-poll][{}] firing toast id={} subject={:?}",
        account_id,
        msg.id,
        msg.subject
    );
    forward_synthetic_notification(app, account_id, "gmail", title, body);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_returns_true_for_new_id_and_false_for_dup() {
        let mut s = SeenState::default();
        assert!(s.insert("a".into()));
        assert!(!s.insert("a".into()));
        assert!(s.insert("b".into()));
    }

    #[test]
    fn fifo_evicts_oldest_at_cap() {
        let mut s = SeenState::default();
        for i in 0..SEEN_SET_CAP {
            assert!(s.insert(format!("id{i}")));
        }
        assert_eq!(s.set.len(), SEEN_SET_CAP);

        // Insert one more — oldest ("id0") must be evicted.
        assert!(s.insert("overflow".into()));
        assert_eq!(s.set.len(), SEEN_SET_CAP);
        assert!(!s.set.contains("id0"));
        assert!(s.set.contains("overflow"));
    }

    #[test]
    fn round_trip_through_disk_format_preserves_order() {
        let mut s = SeenState::default();
        s.insert("first".into());
        s.insert("second".into());
        s.insert("third".into());

        let on_disk = s.to_disk();
        let s2 = SeenState::from_disk(on_disk);

        assert_eq!(
            s2.order.iter().cloned().collect::<Vec<_>>(),
            vec!["first", "second", "third"]
        );
        assert_eq!(s2.set.len(), 3);
    }

    #[test]
    fn save_and_load_round_trip_via_tempfile() {
        let dir = tempfile_dir();
        let path = dir.join("gmail").join("acct-x").join("seen.json");

        let mut s = SeenState::default();
        s.insert("alpha".into());
        s.insert("beta".into());
        save_seen(&path, &s);

        let reloaded = load_seen(&path);
        assert_eq!(reloaded.order.len(), 2);
        assert!(reloaded.set.contains("alpha"));
        assert!(reloaded.set.contains("beta"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_returns_empty_when_path_missing() {
        let dir = tempfile_dir();
        let path = dir.join("does-not-exist.json");
        let s = load_seen(&path);
        assert_eq!(s.set.len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_returns_empty_on_corrupt_json() {
        let dir = tempfile_dir();
        let path = dir.join("corrupt.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not valid json {{{").unwrap();
        let s = load_seen(&path);
        assert_eq!(s.set.len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn tempfile_dir() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "openhuman-gmail-notify-test-{}",
            std::process::id()
        ));
        p.push(format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        p
    }
}

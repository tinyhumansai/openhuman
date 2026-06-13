//! Workspace-backed Telegram chat → thread bindings for remote control.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const STORE_FILE: &str = "state/telegram_remote_sessions.json";
const LOG_PREFIX: &str = "[telegram-remote]";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct TelegramChatBinding {
    pub(crate) thread_id: String,
    pub(crate) sender_key: String,
    pub(crate) updated_at: String,
    /// Human-readable title captured at `/new` time so `/status` can display it
    /// without listing all threads (O(1) instead of O(n) disk reads).
    #[serde(default)]
    pub(crate) title: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct TelegramSessionStoreFile {
    bindings: HashMap<String, TelegramChatBinding>,
    #[serde(default)]
    busy_reply_targets: HashMap<String, bool>,
}

pub(crate) struct TelegramSessionStore {
    file: TelegramSessionStoreFile,
    path: PathBuf,
}

impl TelegramSessionStore {
    pub(crate) fn load(workspace_dir: &Path) -> anyhow::Result<Self> {
        let path = workspace_dir.join(STORE_FILE);
        let file = if path.exists() {
            let raw = std::fs::read_to_string(&path)?;
            serde_json::from_str(&raw).unwrap_or_else(|error| {
                tracing::warn!(
                    "{LOG_PREFIX} corrupt session store at {}: {error}; resetting",
                    path.display()
                );
                TelegramSessionStoreFile::default()
            })
        } else {
            TelegramSessionStoreFile::default()
        };
        tracing::debug!(
            "{LOG_PREFIX} loaded session store bindings={} busy={}",
            file.bindings.len(),
            file.busy_reply_targets.len()
        );
        Ok(Self { file, path })
    }

    pub(crate) fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(&self.file)?;
        std::fs::write(&self.path, raw)?;
        Ok(())
    }

    pub(crate) fn binding(&self, reply_target: &str) -> Option<&TelegramChatBinding> {
        self.file.bindings.get(reply_target)
    }

    pub(crate) fn set_binding(
        &mut self,
        reply_target: &str,
        thread_id: String,
        sender_key: String,
        title: Option<String>,
    ) {
        let updated_at = chrono::Utc::now().to_rfc3339();
        self.file.bindings.insert(
            reply_target.to_string(),
            TelegramChatBinding {
                thread_id,
                sender_key,
                updated_at,
                title,
            },
        );
    }

    pub(crate) fn set_busy(&mut self, reply_target: &str, busy: bool) {
        if busy {
            self.file
                .busy_reply_targets
                .insert(reply_target.to_string(), true);
        } else {
            self.file.busy_reply_targets.remove(reply_target);
        }
    }

    pub(crate) fn is_busy(&self, reply_target: &str) -> bool {
        self.file
            .busy_reply_targets
            .get(reply_target)
            .copied()
            .unwrap_or(false)
    }
}

static STORE: std::sync::OnceLock<std::sync::Mutex<Option<TelegramSessionStore>>> =
    std::sync::OnceLock::new();

/// Read-write accessor: runs `f` against the cached store, then flushes to disk.
/// Use for operations that mutate state (e.g. `set_binding`, `set_busy`).
pub(crate) fn with_store<F, R>(workspace_dir: &Path, f: F) -> anyhow::Result<R>
where
    F: FnOnce(&mut TelegramSessionStore) -> anyhow::Result<R>,
{
    let lock = STORE.get_or_init(|| std::sync::Mutex::new(None));
    let mut guard = lock.lock().expect("telegram session store mutex poisoned");
    let expected_path = workspace_dir.join(STORE_FILE);
    let needs_load = guard
        .as_ref()
        .map(|store| store.path != expected_path)
        .unwrap_or(true);
    if needs_load {
        *guard = Some(TelegramSessionStore::load(workspace_dir)?);
    }
    let store = guard.as_mut().expect("store initialized");
    let result = f(store)?;
    store.save()?;
    Ok(result)
}

/// Read-only accessor: runs `f` against the cached store but does **not** flush
/// to disk. Use for operations that only read state (e.g. `binding`, `is_busy`)
/// to avoid unnecessary serialization and disk I/O on every query.
pub(crate) fn with_store_read<F, R>(workspace_dir: &Path, f: F) -> anyhow::Result<R>
where
    F: FnOnce(&TelegramSessionStore) -> anyhow::Result<R>,
{
    let lock = STORE.get_or_init(|| std::sync::Mutex::new(None));
    let mut guard = lock.lock().expect("telegram session store mutex poisoned");
    let expected_path = workspace_dir.join(STORE_FILE);
    let needs_load = guard
        .as_ref()
        .map(|store| store.path != expected_path)
        .unwrap_or(true);
    if needs_load {
        *guard = Some(TelegramSessionStore::load(workspace_dir)?);
    }
    let store = guard.as_ref().expect("store initialized");
    f(store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn round_trip_binding_and_busy_flag() {
        let dir = tempdir().expect("tempdir");
        let mut store = TelegramSessionStore::load(dir.path()).expect("load");
        store.set_binding(
            "12345",
            "thread-abc".into(),
            "telegram_alice_12345".into(),
            Some("My Session".into()),
        );
        store.set_busy("12345", true);
        store.save().expect("save");

        let reloaded = TelegramSessionStore::load(dir.path()).expect("reload");
        let binding = reloaded.binding("12345").expect("binding");
        assert_eq!(binding.thread_id, "thread-abc");
        assert_eq!(binding.title.as_deref(), Some("My Session"));
        assert!(reloaded.is_busy("12345"));
    }

    #[test]
    fn corrupt_store_resets_and_clearing_busy_removes_flag() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join(STORE_FILE);
        std::fs::create_dir_all(path.parent().expect("state dir")).expect("state dir");
        std::fs::write(&path, "{ not valid json").expect("write corrupt store");

        let mut store = TelegramSessionStore::load(dir.path()).expect("load corrupt store");
        assert!(store.binding("12345").is_none());

        store.set_busy("12345", true);
        assert!(store.is_busy("12345"));
        store.set_busy("12345", false);
        assert!(!store.is_busy("12345"));
        store.save().expect("save reset store");

        let raw = std::fs::read_to_string(path).expect("read saved store");
        assert!(raw.contains("\"bindings\""));
        assert!(!raw.contains("12345"));
    }

    /// Tests `with_store` workspace-change detection by using `TelegramSessionStore`
    /// directly for cross-workspace assertions — avoids races with the process-global
    /// `STORE` singleton when tests run in parallel.
    #[test]
    fn store_isolates_bindings_across_workspaces() {
        let first = tempdir().expect("first tempdir");
        let second = tempdir().expect("second tempdir");

        // Write binding into first workspace directly (no global singleton).
        let mut store_a = TelegramSessionStore::load(first.path()).expect("load first");
        store_a.set_binding("chat-a", "thread-a".into(), "telegram_a".into(), None);
        store_a.save().expect("save first");

        // Write binding into second workspace directly.
        let mut store_b = TelegramSessionStore::load(second.path()).expect("load second");
        assert!(
            store_b.binding("chat-a").is_none(),
            "second workspace must not see first workspace's binding"
        );
        store_b.set_binding("chat-b", "thread-b".into(), "telegram_b".into(), None);
        store_b.save().expect("save second");

        let first_store = TelegramSessionStore::load(first.path()).expect("reload first");
        let second_store = TelegramSessionStore::load(second.path()).expect("reload second");
        assert_eq!(
            first_store
                .binding("chat-a")
                .map(|binding| binding.thread_id.as_str()),
            Some("thread-a")
        );
        assert_eq!(
            second_store
                .binding("chat-b")
                .map(|binding| binding.thread_id.as_str()),
            Some("thread-b")
        );
    }
}

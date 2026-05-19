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
    ) {
        let updated_at = chrono::Utc::now().to_rfc3339();
        self.file.bindings.insert(
            reply_target.to_string(),
            TelegramChatBinding {
                thread_id,
                sender_key,
                updated_at,
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

pub(crate) fn with_store<F, R>(workspace_dir: &Path, f: F) -> anyhow::Result<R>
where
    F: FnOnce(&mut TelegramSessionStore) -> anyhow::Result<R>,
{
    let lock = STORE.get_or_init(|| std::sync::Mutex::new(None));
    let mut guard = lock.lock().expect("telegram session store mutex poisoned");
    if guard.is_none() {
        *guard = Some(TelegramSessionStore::load(workspace_dir)?);
    }
    let store = guard.as_mut().expect("store initialized");
    let result = f(store)?;
    store.save()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn round_trip_binding_and_busy_flag() {
        let dir = tempdir().expect("tempdir");
        let mut store = TelegramSessionStore::load(dir.path()).expect("load");
        store.set_binding("12345", "thread-abc".into(), "telegram_alice_12345".into());
        store.set_busy("12345", true);
        store.save().expect("save");

        let reloaded = TelegramSessionStore::load(dir.path()).expect("reload");
        let binding = reloaded.binding("12345").expect("binding");
        assert_eq!(binding.thread_id, "thread-abc");
        assert!(reloaded.is_busy("12345"));
    }
}

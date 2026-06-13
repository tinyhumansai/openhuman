//! Per-thread CC session UUID persistence.
//!
//! The `claude` CLI's `--resume <uuid>` only reuses a server-side session
//! if we pass it the same UUIDv4 we used the first time. We map an
//! OpenHuman thread id → CC session UUID in a JSON file under the
//! workspace.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreFile {
    /// thread_id → CC session uuid (v4)
    sessions: HashMap<String, String>,
}

/// Disk-backed session store. Cheap to clone — it's `Arc`-shareable via
/// the holding `ClaudeCodeProvider`.
#[derive(Debug)]
pub struct SessionStore {
    path: PathBuf,
    inner: Mutex<StoreFile>,
}

impl SessionStore {
    /// Open (or initialize) the session store at `workspace/claude-code-sessions.json`.
    pub fn open(workspace_dir: &Path) -> Self {
        let path = workspace_dir.join("claude-code-sessions.json");
        let inner = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<StoreFile>(&s).ok())
            .unwrap_or_default();
        Self {
            path,
            inner: Mutex::new(inner),
        }
    }

    /// Lookup an existing CC session UUID for `thread_id`.
    pub fn get(&self, thread_id: &str) -> Option<String> {
        let guard = self.inner.lock().expect("session store mutex poisoned");
        guard.sessions.get(thread_id).cloned()
    }

    /// Persist a thread → UUID mapping.
    pub fn set(&self, thread_id: &str, uuid: &str) -> std::io::Result<()> {
        let mut guard = self.inner.lock().expect("session store mutex poisoned");
        guard
            .sessions
            .insert(thread_id.to_string(), uuid.to_string());
        let serialized = serde_json::to_string_pretty(&*guard).map_err(std::io::Error::other)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, serialized)
    }
}

/// Random RFC-4122 v4 UUID, formatted lower-case with hyphens.
pub fn generate_uuid_v4() -> String {
    use rand::RngExt as _;
    let mut bytes = [0u8; 16];
    rand::rng().fill(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant 10
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    )
}

/// CC accepts only RFC-4122 v4. Older stores might carry pre-v4 strings;
/// we treat those as missing and regenerate.
pub fn is_uuid_v4(s: &str) -> bool {
    let s = s.as_bytes();
    if s.len() != 36 {
        return false;
    }
    let hyphens = [8, 13, 18, 23];
    for (i, b) in s.iter().enumerate() {
        let is_hyphen = hyphens.contains(&i);
        if is_hyphen {
            if *b != b'-' {
                return false;
            }
        } else if !b.is_ascii_hexdigit() {
            return false;
        }
    }
    // version nibble (index 14) must be '4'; variant nibble (index 19)
    // must be one of 8/9/a/b
    s[14] == b'4' && matches!(s[19], b'8' | b'9' | b'a' | b'b' | b'A' | b'B')
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn uuid_v4_format() {
        let id = generate_uuid_v4();
        assert!(is_uuid_v4(&id), "generated id should be v4: {id}");
    }

    #[test]
    fn rejects_non_v4() {
        assert!(!is_uuid_v4("not-a-uuid"));
        assert!(!is_uuid_v4("cc_abc123"));
        // version 1 uuid (nibble at 14 is '1')
        assert!(!is_uuid_v4("00000000-0000-1000-8000-000000000000"));
    }

    #[test]
    fn roundtrip_set_and_get() {
        let dir = tempdir().unwrap();
        let store = SessionStore::open(dir.path());
        assert!(store.get("thread_a").is_none());
        store.set("thread_a", "abc").unwrap();
        let reopened = SessionStore::open(dir.path());
        assert_eq!(reopened.get("thread_a").as_deref(), Some("abc"));
    }
}

//! Per-user, per-toolkit scope preferences.
//!
//! For each Composio toolkit a user has connected (or could connect),
//! we store a [`UserScopePref`] that records whether the agent is
//! allowed to call **read**, **write**, and / or **admin**-classified
//! actions for that toolkit. Defaults are `read=true, write=true,
//! admin=false` — the agent can use the integration productively out of
//! the box, but destructive / permission-changing actions require
//! explicit opt-in.
//!
//! Storage uses the same KV surface as [`super::sync_state`]
//! (`MemoryClient::kv_get` / `kv_set`) under a dedicated namespace so
//! prefs survive process restarts without any extra file management.

use serde::{Deserialize, Serialize};

use crate::openhuman::memory_store::MemoryClientRef;

use super::tool_scope::ToolScope;

/// KV namespace for scope prefs. Separate from `composio-sync-state` so
/// the two never collide.
const KV_NAMESPACE: &str = "composio-user-scopes";

/// Per-toolkit scope preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserScopePref {
    #[serde(default = "default_true")]
    pub read: bool,
    #[serde(default = "default_true")]
    pub write: bool,
    #[serde(default)]
    pub admin: bool,
}

fn default_true() -> bool {
    true
}

impl Default for UserScopePref {
    fn default() -> Self {
        Self {
            read: true,
            write: true,
            admin: false,
        }
    }
}

impl UserScopePref {
    /// Returns `true` if the given scope is enabled in this preference.
    pub fn allows(&self, scope: ToolScope) -> bool {
        match scope {
            ToolScope::Read => self.read,
            ToolScope::Write => self.write,
            ToolScope::Admin => self.admin,
        }
    }
}

fn kv_key(toolkit: &str) -> String {
    toolkit.trim().to_ascii_lowercase()
}

/// Load the scope pref for `toolkit`. Returns the default
/// (`read+write`, no `admin`) when nothing is stored or when the KV
/// store can't be reached — the agent should always be able to use
/// connected integrations productively, even if pref storage is
/// temporarily unavailable.
pub async fn load(memory: &MemoryClientRef, toolkit: &str) -> UserScopePref {
    let key = kv_key(toolkit);
    if key.is_empty() {
        return UserScopePref::default();
    }
    match memory.kv_get(Some(KV_NAMESPACE), &key).await {
        Ok(Some(value)) => match serde_json::from_value::<UserScopePref>(value) {
            Ok(pref) => {
                tracing::debug!(
                    toolkit = %key,
                    read = pref.read,
                    write = pref.write,
                    admin = pref.admin,
                    "[composio][scopes] pref loaded"
                );
                pref
            }
            Err(e) => {
                tracing::warn!(
                    toolkit = %key,
                    error = %e,
                    "[composio][scopes] pref deserialize failed, falling back to default"
                );
                UserScopePref::default()
            }
        },
        Ok(None) => {
            tracing::debug!(
                toolkit = %key,
                "[composio][scopes] no pref stored, using default (read+write)"
            );
            UserScopePref::default()
        }
        Err(e) => {
            tracing::warn!(
                toolkit = %key,
                error = %e,
                "[composio][scopes] kv_get failed, falling back to default"
            );
            UserScopePref::default()
        }
    }
}

/// Persist a scope pref for `toolkit`.
pub async fn save(
    memory: &MemoryClientRef,
    toolkit: &str,
    pref: UserScopePref,
) -> Result<(), String> {
    let key = kv_key(toolkit);
    if key.is_empty() {
        return Err("user_scopes: toolkit must not be empty".to_string());
    }
    let value = serde_json::to_value(pref)
        .map_err(|e| format!("[composio][scopes] serialize failed: {e}"))?;
    memory.kv_set(Some(KV_NAMESPACE), &key, &value).await?;
    tracing::info!(
        toolkit = %key,
        read = pref.read,
        write = pref.write,
        admin = pref.admin,
        "[composio][scopes] pref saved"
    );
    Ok(())
}

/// Best-effort load that resolves the active memory client itself. Used
/// from the meta-tool layer where we don't have a `MemoryClientRef` in
/// scope. Falls back to the default pref when memory isn't initialised.
pub async fn load_or_default(toolkit: &str) -> UserScopePref {
    match crate::openhuman::memory::global::client_if_ready() {
        Some(client) => load(&client, toolkit).await,
        None => {
            // Match the normalized key form `load()` logs so traces
            // grouped by `key` correlate across both code paths.
            let key = kv_key(toolkit);
            tracing::debug!(
                toolkit = %toolkit,
                key = %key,
                "[composio][scopes] memory not ready, using default pref"
            );
            UserScopePref::default()
        }
    }
}

#[cfg(test)]
#[path = "user_scopes_tests.rs"]
mod tests;

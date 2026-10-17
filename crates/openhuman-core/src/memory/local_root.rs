//! The engine scope root of a signed-out (local) session.
//!
//! A local session's user dir is named after the machine
//! (`users/local-<hostname>/`, `session_support::local_session_user_id`), and
//! its root used to be a hash of that name: `user:local-<sha256[..8]>`. Two
//! machines with the same hostname on one CortexDB then got the same root
//! and merged memories, and a hostname is a name even when hashed
//! (memory audit 01-scopes F2).
//!
//! The root is now `user:local-<install id>`: a random UUID minted once and
//! recorded in the `memory`/`state`/`local_root` storage value (once
//! `<workspace>/memory/local_root.json`). Nothing about the
//! machine goes into it.
//!
//! An install whose memory already lives under the hostname-derived root
//! keeps it: the first resolution records *that* root instead of minting
//! one, so no memory is orphaned. Memory lives under the user root only once
//! the layout migration has begun (its state file exists) or finished
//! (`[memory] layout = "v3"`); before that it is in the legacy tree, which the
//! migration later moves under whatever root is recorded.
//!
//! The record is written once and never replaced. Two processes racing to
//! write it both read back the one that landed first. If it cannot be
//! written or read, the session has no root and memory stays off rather than
//! landing under a root the next launch would not find.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};

use crate::config::Config;

/// The prefix every local root carries.
pub const LOCAL_ROOT_PREFIX: &str = "user:local-";

/// Where a root came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// A fresh random install id.
    Minted,
    /// The root an existing install already kept memory under, derived from
    /// its user dir (hostname) name before install ids existed.
    Legacy,
}

/// The persisted record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalRoot {
    /// The engine scope root, `user:local-<id>`.
    pub root: String,
    /// Where it came from.
    pub origin: Origin,
}

/// Resolved roots by record path, so the engine binding, called often, reads
/// the file once per process.
static CACHE: LazyLock<Mutex<HashMap<PathBuf, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

const STATE: &str = "local_root";

/// The record's key in the memory state store; `workspace_dir` also names the
/// legacy `memory/local_root.json` it is imported from.
#[must_use]
pub fn path(workspace_dir: &Path) -> PathBuf {
    super::state_store::legacy_path(workspace_dir, STATE)
}

/// The root of the local session `user_id` whose config is `config`: the
/// recorded one, else a newly recorded one ([`Origin::Legacy`] when memory
/// already lives under the old hostname-derived root, [`Origin::Minted`]
/// otherwise). `None` when the record cannot be read or written.
#[must_use]
pub fn resolve(config: &Config, user_id: &str) -> Option<String> {
    let file = path(&config.workspace_dir);
    if let Some(root) = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&file)
    {
        return Some(root.clone());
    }
    let record = match read(&config.workspace_dir) {
        Ok(Some(record)) => record,
        Ok(None) => {
            let fresh = if has_memory_under_legacy_root(config) {
                LocalRoot {
                    root: legacy_root(user_id),
                    origin: Origin::Legacy,
                }
            } else {
                LocalRoot {
                    root: format!("{LOCAL_ROOT_PREFIX}{}", uuid::Uuid::new_v4().simple()),
                    origin: Origin::Minted,
                }
            };
            match record_once(&config.workspace_dir, &fresh) {
                Ok(landed) => {
                    tracing::info!(
                        origin = ?landed.origin,
                        "[memory:local_root] local session root recorded"
                    );
                    landed
                }
                Err(error) => {
                    tracing::warn!(%error, "[memory:local_root] local root not recorded; memory stays off");
                    return None;
                }
            }
        }
        Err(error) => {
            tracing::warn!(%error, "[memory:local_root] local root record unreadable; memory stays off");
            return None;
        }
    };
    CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(file, record.root.clone());
    Some(record.root)
}

/// The root a local session `user_id` had before install ids:
/// `user:local-<first 8 bytes of sha256(user_id), hex>`. Only ever recorded
/// for an install that already keeps memory under it.
#[must_use]
pub fn legacy_root(user_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest: String = Sha256::digest(user_id.as_bytes())
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{LOCAL_ROOT_PREFIX}{digest}")
}

/// Whether `config`'s memory may already sit under its legacy root: the
/// layout migration (which copies into the user root) has started or
/// finished.
fn has_memory_under_legacy_root(config: &Config) -> bool {
    super::scope::layout_is_v3(config)
        || super::layout_migration::state::exists(&config.workspace_dir)
}

fn valid(record: &LocalRoot) -> std::io::Result<()> {
    let id = record.root.strip_prefix(LOCAL_ROOT_PREFIX).unwrap_or("");
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(std::io::Error::other(
            "the recorded local root is malformed",
        ));
    }
    Ok(())
}

fn read(workspace_dir: &Path) -> anyhow::Result<Option<LocalRoot>> {
    let record: Option<LocalRoot> = super::state_store::load_strict(workspace_dir, STATE)?;
    if let Some(record) = &record {
        valid(record)?;
    }
    Ok(record)
}

/// Writes `record` unless one is already stored, and returns the one that is:
/// the insert fails rather than replace another process's record.
fn record_once(workspace_dir: &Path, record: &LocalRoot) -> anyhow::Result<LocalRoot> {
    let landed = super::state_store::record_once(workspace_dir, STATE, record)?;
    valid(&landed)?;
    Ok(landed)
}

#[cfg(test)]
#[path = "local_root_tests.rs"]
mod tests;

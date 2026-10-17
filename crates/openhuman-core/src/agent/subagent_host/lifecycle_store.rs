//! The sub-agent lifecycle records on the storage port.
//!
//! One record per scoped task key (`record/<key component>` in `tsd.kv`, owner
//! `subagents`, namespace `lifecycle`) holds the key's pause checkpoint and
//! its terminal outcome together, so "a terminal closes the pause" is one
//! compare-and-swap rather than two files under a lock. A second value per task id
//! (`index/<task component>`) lists the scoped keys that share the id.
//!
//! Before schema v2 these were `<workspace>/.openhuman/subagent_checkpoints/
//! lifecycle/<component>.{pause,terminal}.json` and `task-<component>.index.json`
//! guarded by `flock` files. [`LifecycleStore::open`] imports them once per
//! backend and scope (`tsd.migrations`) and leaves the files in place.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tinyagents_orchestration::subagent::{SubagentError, SubagentOutcome, SubagentTaskKey};

use crate::agent::subagent_host::SubagentCheckpointData;
use crate::storage::state_kv::StateKv;

const OWNER: &str = "subagents";
const NS: &str = "lifecycle";
const IMPORT: &str = "lifecycle_dir";

/// A task's terminal outcome with the host facts the neutral outcome lacks.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct TerminalRecord {
    pub(super) outcome: SubagentOutcome,
    #[serde(default)]
    pub(super) agent_id: String,
    #[serde(default)]
    pub(super) elapsed_ms: u64,
}

/// A terminal file as older builds wrote it: the record, or the bare outcome.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum CompatibleTerminalRecord {
    Current(TerminalRecord),
    Legacy(SubagentOutcome),
}

impl From<CompatibleTerminalRecord> for TerminalRecord {
    fn from(record: CompatibleTerminalRecord) -> Self {
        match record {
            CompatibleTerminalRecord::Current(record) => record,
            CompatibleTerminalRecord::Legacy(outcome) => Self {
                outcome,
                agent_id: String::new(),
                elapsed_ms: 0,
            },
        }
    }
}

/// Everything durable about one scoped task key.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct LifecycleRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) pause: Option<SubagentCheckpointData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) terminal: Option<TerminalRecord>,
}

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct TaskIndex {
    #[serde(default)]
    keys: Vec<SubagentTaskKey>,
}

fn failure(error: impl std::fmt::Display) -> SubagentError {
    SubagentError::Persistence(format!("{error:#}"))
}

fn hex(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A task key normally contains short UUID-like identifiers, for which the
/// reversible encoding preserves the old file names. A background child may
/// inherit a rendered parent identifier, however; that exceptional form is
/// hashed while keeping every key field in the digest input so distinct
/// lifecycle scopes cannot alias.
fn bounded(encoded: String) -> String {
    if encoded.len() <= 180 {
        encoded
    } else {
        format!("sha256-{:x}", Sha256::digest(encoded.as_bytes()))
    }
}

pub(super) fn key_component(key: &SubagentTaskKey) -> String {
    bounded(format!(
        "{}-{}-{}-{}",
        hex(&key.root_run_id),
        hex(&key.parent_run_id),
        hex(key.thread_id.as_deref().unwrap_or("")),
        hex(&key.task_id),
    ))
}

pub(super) fn task_component(task_id: &str) -> String {
    bounded(hex(task_id))
}

/// The workspace a checkpoint directory belongs to: the one above
/// `.openhuman/subagent_checkpoints`, else the directory itself.
fn workspace_of(checkpoint_dir: &Path) -> PathBuf {
    let tail = checkpoint_dir.file_name().and_then(|name| name.to_str());
    let parent = checkpoint_dir.parent();
    if tail == Some("subagent_checkpoints")
        && parent
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            == Some(".openhuman")
    {
        if let Some(workspace) = parent.and_then(Path::parent) {
            return workspace.to_path_buf();
        }
    }
    checkpoint_dir.to_path_buf()
}

/// The lifecycle store of one checkpoint directory's workspace.
pub(super) struct LifecycleStore {
    kv: StateKv,
}

impl LifecycleStore {
    /// Opens the store for `checkpoint_dir`, importing its legacy lifecycle
    /// files once.
    pub(super) fn open(checkpoint_dir: &Path) -> Result<Self, SubagentError> {
        let kv = StateKv::open(&workspace_of(checkpoint_dir), OWNER, NS).map_err(failure)?;
        let legacy = checkpoint_dir.join("lifecycle");
        kv.import_once(IMPORT, || import_legacy(&kv, &legacy))
            .map_err(failure)?;
        Ok(Self { kv })
    }

    fn record_name(key: &SubagentTaskKey) -> String {
        format!("record/{}", key_component(key))
    }

    fn index_name(task_id: &str) -> String {
        format!("index/{}", task_component(task_id))
    }

    /// The record of `key` (empty when nothing was written).
    pub(super) fn record(&self, key: &SubagentTaskKey) -> Result<LifecycleRecord, SubagentError> {
        Ok(self
            .kv
            .get(&Self::record_name(key))
            .map_err(failure)?
            .unwrap_or_default())
    }

    /// Changes the record of `key` under compare-and-swap; `change` may run
    /// more than once, and returns its result and whether it changed the record.
    pub(super) fn update<R: Send + 'static>(
        &self,
        key: &SubagentTaskKey,
        change: impl Fn(&mut LifecycleRecord) -> (R, bool) + Send + Sync + 'static,
    ) -> Result<R, SubagentError> {
        self.kv
            .update(&Self::record_name(key), change)
            .map_err(failure)
    }

    /// The scoped keys that share `task_id`.
    pub(super) fn index_keys(&self, task_id: &str) -> Result<Vec<SubagentTaskKey>, SubagentError> {
        Ok(self
            .kv
            .get::<TaskIndex>(&Self::index_name(task_id))
            .map_err(failure)?
            .unwrap_or_default()
            .keys)
    }

    pub(super) fn add_index_key(&self, key: &SubagentTaskKey) -> Result<(), SubagentError> {
        let key = key.clone();
        self.kv
            .update::<TaskIndex, ()>(&Self::index_name(&key.task_id), {
                let key = key.clone();
                move |index| {
                    if index.keys.contains(&key) {
                        return ((), false);
                    }
                    index.keys.push(key.clone());
                    ((), true)
                }
            })
            .map_err(failure)
    }

    pub(super) fn remove_index_key(&self, key: &SubagentTaskKey) -> Result<(), SubagentError> {
        let key = key.clone();
        self.kv
            .update::<TaskIndex, ()>(&Self::index_name(&key.task_id), {
                let key = key.clone();
                move |index| {
                    let before = index.keys.len();
                    index.keys.retain(|candidate| candidate != &key);
                    ((), index.keys.len() != before)
                }
            })
            .map_err(failure)
    }
}

/// Copies the legacy lifecycle files of `dir`, never replacing a record that
/// already holds the part a file carries.
fn import_legacy(kv: &StateKv, dir: &Path) -> anyhow::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let mut imported = 0usize;
    for entry in entries {
        let path = entry?.path();
        let Some(file) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("[subagent:lifecycle] legacy {file} unreadable ({error}); skipped");
                continue;
            }
        };
        if let Some(component) = file.strip_suffix(".pause.json") {
            match serde_json::from_slice::<SubagentCheckpointData>(&bytes) {
                Ok(pause) => {
                    kv.update::<LifecycleRecord, ()>(&format!("record/{component}"), move |r| {
                        if r.pause.is_none() {
                            r.pause = Some(pause.clone());
                            ((), true)
                        } else {
                            ((), false)
                        }
                    })?;
                    imported += 1;
                }
                Err(error) => log::warn!(
                    "[subagent:lifecycle] legacy pause {component} unparsable ({error}); skipped"
                ),
            }
        } else if let Some(component) = file.strip_suffix(".terminal.json") {
            match serde_json::from_slice::<CompatibleTerminalRecord>(&bytes) {
                Ok(terminal) => {
                    let terminal = TerminalRecord::from(terminal);
                    kv.update::<LifecycleRecord, ()>(&format!("record/{component}"), move |r| {
                        if r.terminal.is_none() {
                            r.terminal = Some(terminal.clone());
                            ((), true)
                        } else {
                            ((), false)
                        }
                    })?;
                    imported += 1;
                }
                Err(error) => log::warn!(
                    "[subagent:lifecycle] legacy terminal {component} unparsable ({error}); skipped"
                ),
            }
        } else if let Some(component) = file
            .strip_prefix("task-")
            .and_then(|rest| rest.strip_suffix(".index.json"))
        {
            let value: serde_json::Value = match serde_json::from_slice(&bytes) {
                Ok(value) => value,
                Err(error) => {
                    log::warn!("[subagent:lifecycle] legacy index {component} unparsable ({error})");
                    continue;
                }
            };
            // The short-lived single-key format of the first lifecycle
            // extraction is read as a one-key list.
            let keys: Vec<SubagentTaskKey> = value
                .get("keys")
                .cloned()
                .and_then(|keys| serde_json::from_value(keys).ok())
                .or_else(|| {
                    value
                        .get("key")
                        .cloned()
                        .and_then(|key| serde_json::from_value(key).ok())
                        .map(|key| vec![key])
                })
                .unwrap_or_default();
            kv.update::<TaskIndex, ()>(&format!("index/{component}"), move |index| {
                let mut changed = false;
                for key in &keys {
                    if !index.keys.contains(key) {
                        index.keys.push(key.clone());
                        changed = true;
                    }
                }
                ((), changed)
            })?;
            imported += 1;
        }
    }
    log::debug!("[subagent:lifecycle] imported {imported} legacy lifecycle files");
    Ok(())
}

#[cfg(test)]
#[path = "lifecycle_store_tests.rs"]
mod tests;

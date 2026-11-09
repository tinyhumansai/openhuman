//! Migration from retired OpenHuman sub-agent checkpoints.
//!
//! The host's former `subagents/lifecycle` KV values and lifecycle JSON
//! files are compatibility sources. This adapter reads their product fields,
//! imports missing file records once per backend/scope, and converts them
//! into TinyAgents' neutral document records without changing source files
//! or existing KV values.
//!
//! TinyAgents' `DocumentSubagentPersistence` owns active pause/terminal
//! transitions, compare-and-swap and live-task lookup. The host retains only
//! legacy schema conversion, product metadata and storage scope selection.

use std::path::{Path, PathBuf};

#[cfg(test)]
use sha2::{Digest, Sha256};
use tinyagents_orchestration::subagent::{SubagentError, SubagentOutcome, SubagentTaskKey};

use crate::agent::subagent_host::SubagentCheckpointData;
use crate::storage::state_kv::StateKv;

const OWNER: &str = "subagents";
const NS: &str = "lifecycle";
const IMPORT: &str = "lifecycle_dir_owner_adoption";

/// A task's terminal outcome with the host facts the neutral outcome lacks.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct TerminalRecord {
    #[serde(default)]
    key: Option<SubagentTaskKey>,
    pub(super) outcome: SubagentOutcome,
    #[serde(default)]
    pub(super) agent_id: String,
    #[serde(default)]
    pub(super) elapsed_ms: u64,
    #[serde(flatten)]
    extensions: std::collections::BTreeMap<String, serde_json::Value>,
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
            CompatibleTerminalRecord::Current(mut record) => {
                if !record.agent_id.is_empty() {
                    record
                        .outcome
                        .extensions
                        .entry("agent_id".into())
                        .or_insert_with(|| serde_json::Value::String(record.agent_id.clone()));
                }
                if record.elapsed_ms != 0 {
                    record
                        .outcome
                        .extensions
                        .entry("elapsed_ms".into())
                        .or_insert_with(|| serde_json::json!(record.elapsed_ms));
                }
                for (name, value) in std::mem::take(&mut record.extensions) {
                    record.outcome.extensions.entry(name).or_insert(value);
                }
                record
            }
            CompatibleTerminalRecord::Legacy(outcome) => Self {
                key: None,
                outcome,
                agent_id: String::new(),
                elapsed_ms: 0,
                extensions: Default::default(),
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

#[cfg(test)]
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
#[cfg(test)]
fn bounded(encoded: String) -> String {
    if encoded.len() <= 180 {
        encoded
    } else {
        format!("sha256-{:x}", Sha256::digest(encoded.as_bytes()))
    }
}

#[cfg(test)]
pub(super) fn key_component(key: &SubagentTaskKey) -> String {
    bounded(format!(
        "{}-{}-{}-{}",
        hex(&key.root_run_id),
        hex(&key.parent_run_id),
        hex(key.thread_id.as_deref().unwrap_or("")),
        hex(&key.task_id),
    ))
}

#[cfg(test)]
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

/// Legacy migration sources and destination documents in one acting scope.
pub(super) struct LifecycleStore {
    kv: StateKv,
    documents: std::sync::Arc<dyn tinystoragedrivers_api::DocumentStore>,
}

impl LifecycleStore {
    /// Opens the store for `checkpoint_dir`, importing its legacy lifecycle
    /// files once.
    pub(super) fn open(checkpoint_dir: &Path) -> Result<Self, SubagentError> {
        Self::with_file_import(checkpoint_dir, true)
    }

    /// A new terminal commit must win over an unadopted legacy file, while a
    /// pause already in the previous document namespace remains authoritative.
    pub(super) fn before_terminal(checkpoint_dir: &Path) -> Result<Self, SubagentError> {
        Self::with_file_import(checkpoint_dir, false)
    }

    fn with_file_import(checkpoint_dir: &Path, import_files: bool) -> Result<Self, SubagentError> {
        let kv = StateKv::open(&workspace_of(checkpoint_dir), OWNER, NS).map_err(failure)?;
        if import_files {
            let legacy = checkpoint_dir.join("lifecycle");
            kv.import_once(IMPORT, |_| import_legacy(&kv, &legacy))
                .map_err(failure)?;
        }
        let opened = crate::storage::local::open("subagents", &workspace_of(checkpoint_dir))
            .map_err(failure)?;
        let scoped = opened.scoped().map_err(failure)?;
        Ok(Self {
            kv,
            documents: std::sync::Arc::clone(scoped.documents()),
        })
    }

    /// Converts the retired host checkpoint schema into the owner's neutral
    /// record namespace. Existing owner records win; source records and files
    /// remain unchanged, and terminal records cannot reopen as pauses.
    pub(super) fn neutral_store(
        &self,
    ) -> Result<tinyagents_orchestration::subagent::DocumentSubagentPersistence, SubagentError>
    {
        use super::persistence::OpenHumanPersistence;
        use tinyagents_orchestration::subagent::{
            DocumentSubagentPersistence, PersistedSubagentPause,
        };
        let store = DocumentSubagentPersistence::new(std::sync::Arc::clone(&self.documents))
            .map_err(failure)?;
        for (name, record) in self
            .kv
            .list::<LifecycleRecord>("record/")
            .map_err(failure)?
        {
            let component = name.strip_prefix("record/").expect("listed record prefix");
            let pause = record.pause.and_then(|checkpoint| {
                let key = checkpoint.task_key.clone()?;
                Some(PersistedSubagentPause {
                    key,
                    outcome: OpenHumanPersistence::checkpoint_outcome(checkpoint),
                    replaces: None,
                })
            });
            let terminal_key = record
                .terminal
                .as_ref()
                .and_then(|record| record.key.clone());
            let terminal = record.terminal.map(|record| {
                let mut outcome = record.outcome;
                if !record.agent_id.is_empty() {
                    outcome
                        .extensions
                        .entry("agent_id".into())
                        .or_insert(serde_json::json!(record.agent_id));
                }
                outcome
                    .extensions
                    .entry("elapsed_ms".into())
                    .or_insert(serde_json::json!(record.elapsed_ms));
                outcome
            });
            // Host files used a reversible component; owner files used a SHA
            // of the serialized key. Pauses carry the key and can normalize.
            if let Some(pause) = pause {
                store.import_record(&pause.key.clone(), Some(pause), terminal)?;
            } else if let Some(key) = terminal_key {
                store.import_record(&key, None, terminal)?;
            } else {
                store.import_record_at(component, None, terminal)?;
            }
        }
        Ok(store)
    }

    #[cfg(test)]
    fn record_name(key: &SubagentTaskKey) -> String {
        format!("record/{}", key_component(key))
    }

    #[cfg(test)]
    fn index_name(task_id: &str) -> String {
        format!("index/{}", task_component(task_id))
    }

    /// The record of `key` (empty when nothing was written).
    #[cfg(test)]
    pub(super) fn record(&self, key: &SubagentTaskKey) -> Result<LifecycleRecord, SubagentError> {
        Ok(self
            .kv
            .get(&Self::record_name(key))
            .map_err(failure)?
            .unwrap_or_default())
    }

    /// Changes the record of `key` under compare-and-swap; `change` may run
    /// more than once, and returns its result and whether it changed the record.
    #[cfg(test)]
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
    #[cfg(test)]
    pub(super) fn index_keys(&self, task_id: &str) -> Result<Vec<SubagentTaskKey>, SubagentError> {
        Ok(self
            .kv
            .get::<TaskIndex>(&Self::index_name(task_id))
            .map_err(failure)?
            .unwrap_or_default()
            .keys)
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
            let checkpoint =
                serde_json::from_slice::<SubagentCheckpointData>(&bytes).or_else(|original| {
                    let persisted = serde_json::from_slice::<
                        tinyagents_orchestration::subagent::PersistedSubagentPause,
                    >(&bytes)
                    .map_err(|_| original)?;
                    super::persistence::OpenHumanPersistence::checkpoint_from_persisted_pause(
                        persisted,
                    )
                    .map_err(<serde_json::Error as serde::de::Error>::custom)
                });
            match checkpoint {
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
                    log::warn!(
                        "[subagent:lifecycle] legacy index {component} unparsable ({error})"
                    );
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

#[cfg(test)]
#[path = "lifecycle_storage_adoption_tests.rs"]
mod adoption_tests;

//! User-consented tiny.place contact pairing for wrapped agent sessions.
//!
//! The tiny.place backend owns the contact graph; this module owns OpenHuman's
//! local consent record for orchestration sessions that are allowed to exchange
//! 1:1 encrypted envelopes.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::config::Config;
use crate::openhuman::tinyplace::ops::{global_state as tinyplace_state, map_err};

const LOG_TARGET: &str = "orchestration_pairing";

static STORE_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingStatus {
    Pending,
    Linked,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingSource {
    UserLink,
    ApprovedRequest,
}

impl PairingSource {
    fn as_str(&self) -> &'static str {
        match self {
            Self::UserLink => "user_link",
            Self::ApprovedRequest => "approved_request",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingRecord {
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub status: PairingStatus,
    pub linked_at: String,
    pub source: PairingSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingSnapshot {
    pub records: Vec<PairingRecord>,
    pub contacts: Value,
    pub requests: Value,
    pub stats: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingActionResult {
    pub record: Option<PairingRecord>,
    pub remote: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairingStore {
    #[serde(default)]
    records: Vec<PairingRecord>,
}

pub async fn list(config: &Config) -> Result<PairingSnapshot, String> {
    log::debug!(target: LOG_TARGET, "[orchestration_pairing] list.entry");
    let records = load_store(&config.workspace_dir).await?.records;
    let client = tinyplace_state().client().await?;
    let contacts: Value = client
        .http()
        .get_agent_auth::<Value>("/contacts", &[("limit".to_string(), "100".to_string())])
        .await
        .map_err(map_err)?;
    let requests: Value = client
        .http()
        .get_agent_auth::<Value>(
            "/contacts/requests",
            &[("limit".to_string(), "100".to_string())],
        )
        .await
        .map_err(map_err)?;
    let stats: Value = client
        .http()
        .get_agent_auth::<Value>("/contacts/stats", &[])
        .await
        .map_err(map_err)?;
    log::debug!(
        target: LOG_TARGET,
        "[orchestration_pairing] list.exit records={}",
        records.len()
    );
    Ok(PairingSnapshot {
        records,
        contacts,
        requests,
        stats,
    })
}

pub async fn link_session(
    config: &Config,
    agent_id: &str,
    label: Option<String>,
) -> Result<PairingActionResult, String> {
    let agent_id = normalize_agent_id(agent_id)?;
    log::debug!(target: LOG_TARGET, "[orchestration_pairing] link.entry agent_id={agent_id}");
    let client = tinyplace_state().client().await?;
    let status = contact_status(&agent_id).await?;
    if status == "blocked" {
        log::warn!(
            target: LOG_TARGET,
            "[orchestration_pairing] link.blocked agent_id={agent_id}"
        );
        return Err("session agent is blocked; unblock before linking".to_string());
    }

    let remote = if status == "accepted" {
        serde_json::json!({ "agentId": agent_id, "status": "accepted" })
    } else {
        client
            .http()
            .post_agent_auth::<Value, ()>(&contact_path(&agent_id, None), None)
            .await
            .map_err(map_err)?
    };
    let record_status = if remote_status(&remote).as_deref() == Some("accepted") {
        PairingStatus::Linked
    } else {
        PairingStatus::Pending
    };
    let record = persist_record(
        &config.workspace_dir,
        agent_id,
        label,
        record_status,
        PairingSource::UserLink,
    )
    .await?;
    publish_pairing_changed(&record);
    log::debug!(target: LOG_TARGET, "[orchestration_pairing] link.exit agent_id={}", record.agent_id);
    Ok(PairingActionResult {
        record: Some(record),
        remote,
    })
}

pub async fn accept_request(
    config: &Config,
    agent_id: &str,
) -> Result<PairingActionResult, String> {
    let agent_id = normalize_agent_id(agent_id)?;
    log::debug!(target: LOG_TARGET, "[orchestration_pairing] accept.entry agent_id={agent_id}");
    let client = tinyplace_state().client().await?;
    let remote: Value = client
        .http()
        .post_agent_auth::<Value, ()>(&contact_path(&agent_id, Some("accept")), None)
        .await
        .map_err(map_err)?;
    let record = persist_record(
        &config.workspace_dir,
        agent_id,
        None,
        PairingStatus::Linked,
        PairingSource::ApprovedRequest,
    )
    .await?;
    publish_pairing_changed(&record);
    log::debug!(target: LOG_TARGET, "[orchestration_pairing] accept.exit agent_id={}", record.agent_id);
    Ok(PairingActionResult {
        record: Some(record),
        remote,
    })
}

pub async fn decline_request(
    config: &Config,
    agent_id: &str,
) -> Result<PairingActionResult, String> {
    let agent_id = normalize_agent_id(agent_id)?;
    log::debug!(target: LOG_TARGET, "[orchestration_pairing] decline.entry agent_id={agent_id}");
    let client = tinyplace_state().client().await?;
    let remote: Value = client
        .http()
        .delete_agent_auth::<Value, ()>(&contact_path(&agent_id, None), None)
        .await
        .map_err(map_err)?;
    remove_record(&config.workspace_dir, &agent_id).await?;
    publish_global(DomainEvent::OrchestrationPairingChanged {
        agent_id: agent_id.clone(),
        status: "removed".to_string(),
        source: "approved_request".to_string(),
    });
    log::debug!(target: LOG_TARGET, "[orchestration_pairing] decline.exit agent_id={agent_id}");
    Ok(PairingActionResult {
        record: None,
        remote,
    })
}

pub async fn block_request(config: &Config, agent_id: &str) -> Result<PairingActionResult, String> {
    let agent_id = normalize_agent_id(agent_id)?;
    log::debug!(target: LOG_TARGET, "[orchestration_pairing] block.entry agent_id={agent_id}");
    let client = tinyplace_state().client().await?;
    let remote: Value = client
        .http()
        .post_agent_auth::<Value, ()>(&contact_path(&agent_id, Some("block")), None)
        .await
        .map_err(map_err)?;
    let record = persist_record(
        &config.workspace_dir,
        agent_id,
        None,
        PairingStatus::Blocked,
        PairingSource::ApprovedRequest,
    )
    .await?;
    publish_pairing_changed(&record);
    log::debug!(target: LOG_TARGET, "[orchestration_pairing] block.exit agent_id={}", record.agent_id);
    Ok(PairingActionResult {
        record: Some(record),
        remote,
    })
}

async fn contact_status(agent_id: &str) -> Result<String, String> {
    let client = tinyplace_state().client().await?;
    let remote: Value = client
        .http()
        .get_agent_auth::<Value>(&contact_path(agent_id, Some("status")), &[])
        .await
        .map_err(map_err)?;
    Ok(remote_status(&remote).unwrap_or_else(|| "none".to_string()))
}

fn remote_status(value: &Value) -> Option<String> {
    value
        .get("status")
        .and_then(Value::as_str)
        .map(str::to_string)
}

async fn persist_record(
    workspace_dir: &Path,
    agent_id: String,
    label: Option<String>,
    status: PairingStatus,
    source: PairingSource,
) -> Result<PairingRecord, String> {
    let store_lock = store_lock(workspace_dir).await;
    let _guard = store_lock.lock().await;
    let mut store = load_store(workspace_dir).await?;
    let record = PairingRecord {
        agent_id,
        label: label.and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }),
        status,
        linked_at: chrono::Utc::now().to_rfc3339(),
        source,
    };
    store
        .records
        .retain(|existing| existing.agent_id != record.agent_id);
    store.records.push(record.clone());
    store.records.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
    save_store(workspace_dir, &store).await?;
    Ok(record)
}

async fn remove_record(workspace_dir: &Path, agent_id: &str) -> Result<(), String> {
    let store_lock = store_lock(workspace_dir).await;
    let _guard = store_lock.lock().await;
    let mut store = load_store(workspace_dir).await?;
    store.records.retain(|record| record.agent_id != agent_id);
    save_store(workspace_dir, &store).await
}

async fn store_lock(workspace_dir: &Path) -> Arc<Mutex<()>> {
    let path = store_path(workspace_dir);
    let mut locks = STORE_LOCKS.lock().await;
    locks
        .entry(path)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

async fn load_store(workspace_dir: &Path) -> Result<PairingStore, String> {
    let path = store_path(workspace_dir);
    match tokio::fs::read(&path).await {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("read orchestration pairing store: {e}")),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(PairingStore::default()),
        Err(err) => Err(format!("read orchestration pairing store: {err}")),
    }
}

/// Agent ids that hold an accepted (linked) local pairing record. Local-only
/// read (no network) — used to gate which DM senders the orchestration layer is
/// allowed to decrypt/ingest, so ordinary human DMs are never consumed. Returns
/// an empty set on any read error (fail-closed: nothing is ingested).
pub(crate) async fn linked_agent_ids(workspace_dir: &Path) -> std::collections::HashSet<String> {
    match load_store(workspace_dir).await {
        Ok(store) => store
            .records
            .into_iter()
            .filter(|record| record.status == PairingStatus::Linked)
            .map(|record| record.agent_id)
            .collect(),
        Err(e) => {
            log::warn!(target: LOG_TARGET, "[orchestration_pairing] linked_agent_ids read failed: {e}");
            std::collections::HashSet::new()
        }
    }
}

async fn save_store(workspace_dir: &Path, store: &PairingStore) -> Result<(), String> {
    let path = store_path(workspace_dir);
    let parent = path
        .parent()
        .ok_or_else(|| "invalid orchestration pairing store path".to_string())?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| format!("create orchestration pairing store dir: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(store)
        .map_err(|e| format!("serialize orchestration pairing store: {e}"))?;
    tokio::fs::write(&tmp, bytes)
        .await
        .map_err(|e| format!("write orchestration pairing store: {e}"))?;
    #[cfg(windows)]
    {
        match tokio::fs::remove_file(&path).await {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(format!(
                    "remove existing orchestration pairing store: {err}"
                ))
            }
        }
    }
    tokio::fs::rename(&tmp, &path)
        .await
        .map_err(|e| format!("replace orchestration pairing store: {e}"))
}

fn store_path(workspace_dir: &Path) -> PathBuf {
    workspace_dir
        .join("agent_orchestration")
        .join("pairings.json")
}

fn normalize_agent_id(agent_id: &str) -> Result<String, String> {
    let trimmed = agent_id.trim();
    if trimmed.is_empty() {
        Err("agentId is required".to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

fn contact_path(agent_id: &str, suffix: Option<&str>) -> String {
    match suffix {
        Some(suffix) => format!("/contacts/{}/{}", encode_path_segment(agent_id), suffix),
        None => format!("/contacts/{}", encode_path_segment(agent_id)),
    }
}

fn encode_path_segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            write!(&mut out, "%{byte:02X}").expect("writing to String cannot fail");
        }
    }
    out
}

fn publish_pairing_changed(record: &PairingRecord) {
    publish_global(DomainEvent::OrchestrationPairingChanged {
        agent_id: record.agent_id.clone(),
        status: serde_json::to_value(&record.status)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| "unknown".to_string()),
        source: record.source.as_str().to_string(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contact_paths_encode_agent_ids() {
        assert_eq!(
            contact_path("agent/with space", Some("status")),
            "/contacts/agent%2Fwith%20space/status"
        );
    }

    #[tokio::test]
    async fn pairing_store_upserts_and_removes_records() {
        let tmp = tempfile::tempdir().unwrap();
        let record = persist_record(
            tmp.path(),
            "@worker".to_string(),
            Some("Worker".to_string()),
            PairingStatus::Pending,
            PairingSource::UserLink,
        )
        .await
        .unwrap();
        assert_eq!(record.agent_id, "@worker");

        let record = persist_record(
            tmp.path(),
            "@worker".to_string(),
            None,
            PairingStatus::Linked,
            PairingSource::ApprovedRequest,
        )
        .await
        .unwrap();
        assert_eq!(record.status, PairingStatus::Linked);

        let store = load_store(tmp.path()).await.unwrap();
        assert_eq!(store.records.len(), 1);
        assert_eq!(store.records[0].source, PairingSource::ApprovedRequest);

        remove_record(tmp.path(), "@worker").await.unwrap();
        let store = load_store(tmp.path()).await.unwrap();
        assert!(store.records.is_empty());
    }

    #[tokio::test]
    async fn pairing_store_rewrites_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        persist_record(
            tmp.path(),
            "@worker".to_string(),
            Some("Worker".to_string()),
            PairingStatus::Pending,
            PairingSource::UserLink,
        )
        .await
        .unwrap();

        persist_record(
            tmp.path(),
            "@worker".to_string(),
            None,
            PairingStatus::Linked,
            PairingSource::ApprovedRequest,
        )
        .await
        .unwrap();

        let store = load_store(tmp.path()).await.unwrap();
        assert_eq!(store.records.len(), 1);
        assert_eq!(store.records[0].status, PairingStatus::Linked);
        assert_eq!(store.records[0].source, PairingSource::ApprovedRequest);
    }

    #[tokio::test]
    async fn pairing_store_serializes_concurrent_mutations() {
        let tmp = tempfile::tempdir().unwrap();
        let mut tasks = Vec::new();

        for index in 0..20 {
            let workspace_dir = tmp.path().to_path_buf();
            tasks.push(tokio::spawn(async move {
                persist_record(
                    &workspace_dir,
                    format!("@worker-{index}"),
                    Some(format!("Worker {index}")),
                    PairingStatus::Linked,
                    PairingSource::ApprovedRequest,
                )
                .await
            }));
        }

        for task in tasks {
            task.await.unwrap().unwrap();
        }

        let store = load_store(tmp.path()).await.unwrap();
        assert_eq!(store.records.len(), 20);
        for index in 0..20 {
            assert!(store
                .records
                .iter()
                .any(|record| record.agent_id == format!("@worker-{index}")));
        }
    }
}

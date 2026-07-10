//! User-consented tiny.place contact pairing for wrapped agent sessions.
//!
//! The tiny.place backend owns the contact graph; this module owns OpenHuman's
//! local consent record for orchestration sessions that are allowed to exchange
//! 1:1 encrypted envelopes.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::config::Config;
use crate::openhuman::orchestration::ingest::resolve_linked_id;
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
            HashSet::new()
        }
    }
}

/// Poll the tiny.place incoming contact-request queue and auto-accept every
/// request whose requester is already a linked (paired) agent — and ONLY those.
/// A request from an agent that is not in the local linked set is deliberately
/// left **pending** for the human to decide: generic auto-accept stays off,
/// because accepting a contact is a trust decision (the relay's own rule is
/// "never auto-accept"). The one exception is the user's *own* already-paired
/// agents.
///
/// Why this is the delivery gate: tiny.place's relay DROPS any DM to a peer that
/// has not accepted a contact request, so a wrapped agent that re-establishes
/// contact (a fresh daemon or reconnect that re-sends its one `contact_add`, or a
/// relay-side contact reset) is otherwise blocked behind a pending request — its
/// session intro (`session_info`) and entire session stream never arrive.
/// Auto-accepting linked agents opens that gate for them only. (A *rotated* key is
/// a different Ed25519 identity, so it is NOT in the linked set and its request is
/// correctly left pending for the human.)
///
/// No per-cycle churn: accepting moves the relay edge from `pending` → `accepted`,
/// and `/contacts/requests` only lists *pending* requests, so an accepted
/// requester drops out of `incoming` and is not re-selected next pass. We also
/// accept under the **canonical** linked id (see [`requesters_to_auto_accept`]),
/// so a base64-form request can't slip past a base58 stored id and re-fire.
///
/// Fail-closed: [`linked_agent_ids`] returns an **empty** set on any pairing-store
/// read error, so a read failure auto-accepts NOTHING (every request is left
/// pending) rather than opening the gate. Returns the number of requests accepted
/// this pass.
pub async fn auto_accept_linked_contact_requests(config: &Config) -> Result<usize, String> {
    let linked = linked_agent_ids(&config.workspace_dir).await;
    // An empty linked set can match nothing — this is also the fail-closed
    // read-error case — so skip the network round-trip entirely.
    if linked.is_empty() {
        return Ok(0);
    }
    let client = tinyplace_state().client().await?;
    // `limit=100` caps one scan (consistent with `list()`); a linked requester
    // beyond the 100th *pending* request waits until earlier ones clear. Fine at
    // expected volumes — a paired fleet is small — revisit with pagination if not.
    let requests: Value = client
        .http()
        .get_agent_auth::<Value>(
            "/contacts/requests",
            &[("limit".to_string(), "100".to_string())],
        )
        .await
        .map_err(map_err)?;
    let to_accept = requesters_to_auto_accept(&incoming_pending_requesters(&requests), &linked);
    log::debug!(
        target: LOG_TARGET,
        "[orchestration_pairing] auto_accept.scan linked={} accept={}",
        linked.len(),
        to_accept.len()
    );
    let mut accepted = 0usize;
    for agent_id in to_accept {
        match accept_request(config, &agent_id).await {
            Ok(_) => {
                accepted += 1;
                log::info!(
                    target: LOG_TARGET,
                    "[orchestration_pairing] auto_accept.linked agent_id={agent_id}"
                );
            }
            Err(e) => log::warn!(
                target: LOG_TARGET,
                "[orchestration_pairing] auto_accept.failed agent_id={agent_id}: {e}"
            ),
        }
    }
    Ok(accepted)
}

/// Requester agent id of every *pending incoming* contact request in a raw
/// `/contacts/requests` response. For an incoming request the counterpart is the
/// requester: prefer the top-level `agentId`, falling back to `contact.requester`
/// (the relay does not always populate `agentId`). Mirrors the frontend
/// `contactAddress` resolution in `orchestrationTabHelpers.ts`. Pure — no IO — so
/// the parse is unit-testable without a live client.
fn incoming_pending_requesters(requests: &Value) -> Vec<String> {
    let Some(incoming) = requests.get("incoming").and_then(Value::as_array) else {
        return Vec::new();
    };
    incoming
        .iter()
        .filter(|view| view.get("status").and_then(Value::as_str) == Some("pending"))
        .filter_map(request_view_requester)
        .collect()
}

/// The counterpart (requester) address of a single incoming contact-request view.
fn request_view_requester(view: &Value) -> Option<String> {
    if let Some(id) = view.get("agentId").and_then(Value::as_str) {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    view.get("contact")
        .and_then(|contact| contact.get("requester"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|requester| !requester.is_empty())
        .map(str::to_string)
}

/// Decide which incoming requesters to auto-accept: exactly those already in the
/// linked-agent set. Requesters not linked are intentionally left for the human.
///
/// Returns each match's **canonical linked id** (via [`resolve_linked_id`]), NOT
/// the raw wire id — so a request that carries the base64 form of a key stored as
/// base58 is accepted/persisted under the existing base58 record rather than
/// spawning a duplicate `Linked` record for the same identity. Pure — the trust
/// gate is unit-testable without any network or store IO.
fn requesters_to_auto_accept(incoming: &[String], linked: &HashSet<String>) -> Vec<String> {
    incoming
        .iter()
        .filter_map(|id| resolve_linked_id(id, linked))
        .collect()
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

    // ── Auto-accept gate (O2) ────────────────────────────────────────────────

    // A real base58 pairing-store address and the base64 Ed25519 key of the SAME
    // 32-byte identity (as seen on the wire), reused from the ingest matcher test.
    const LINKED_BASE58: &str = "7jr5FKYETssD6T1MCzsR4aT4dnjjyJCE2SANYYX1R5vm";
    const LINKED_BASE64: &str = "ZCAAuA+2GVoRrT08Gt8JUVnxnISTelSxnDuyScze334=";
    const UNLINKED_BASE58: &str = "De6RHrMj6eDqX1WBTXk11sks4WXHMaqEX9A6oQ3ZEmsg";

    #[test]
    fn auto_accept_gate_accepts_linked_but_leaves_others_pending() {
        let linked: HashSet<String> = [LINKED_BASE58.to_string()].into_iter().collect();

        // (1) A linked requester is selected for auto-accept.
        assert_eq!(
            requesters_to_auto_accept(&[LINKED_BASE58.to_string()], &linked),
            vec![LINKED_BASE58.to_string()]
        );

        // (2) A non-linked requester is left pending (never selected).
        assert!(
            requesters_to_auto_accept(&[UNLINKED_BASE58.to_string()], &linked).is_empty(),
            "an unlinked requester must be left pending for the human"
        );

        // Mixed batch: only the linked id is accepted, order preserved.
        assert_eq!(
            requesters_to_auto_accept(
                &[
                    UNLINKED_BASE58.to_string(),
                    LINKED_BASE58.to_string(),
                    UNLINKED_BASE58.to_string(),
                ],
                &linked,
            ),
            vec![LINKED_BASE58.to_string()]
        );
    }

    #[test]
    fn auto_accept_gate_unifies_and_canonicalizes_base58_and_base64() {
        // The pairing store keeps the base58 address; a contact request may carry
        // the base64 Ed25519 key. Both are the same identity, so the linked
        // agent's request must still be accepted (the shared matcher unifies the
        // two encodings) — otherwise the e2e intro gate stays shut. Crucially the
        // gate returns the CANONICAL base58 stored id, not the raw base64 wire id,
        // so accepting under it reuses the existing record instead of persisting a
        // duplicate `Linked` row for the same identity.
        let linked: HashSet<String> = [LINKED_BASE58.to_string()].into_iter().collect();
        assert_eq!(
            requesters_to_auto_accept(&[LINKED_BASE64.to_string()], &linked),
            vec![LINKED_BASE58.to_string()],
            "must canonicalize the base64 wire id to the stored base58 id"
        );
    }

    #[test]
    fn auto_accept_gate_accepts_nothing_with_empty_linked_set() {
        // linked_agent_ids() fails closed to an empty set on any store read error;
        // with no linked agents the gate must accept nothing (every request stays
        // pending) rather than opening the contact gate wide.
        let empty = HashSet::new();
        assert!(
            requesters_to_auto_accept(&[LINKED_BASE58.to_string()], &empty).is_empty(),
            "an empty (fail-closed) linked set must auto-accept nothing"
        );
    }

    #[tokio::test]
    async fn auto_accept_fails_closed_on_unreadable_pairing_store() {
        // The read-error path end-to-end (minus network): a corrupt pairing store
        // makes linked_agent_ids() fail closed to an empty set, so the gate cannot
        // auto-accept even a would-be-linked requester.
        let tmp = tempfile::tempdir().unwrap();
        let path = store_path(tmp.path());
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, b"{ this is not valid json")
            .await
            .unwrap();

        let linked = linked_agent_ids(tmp.path()).await;
        assert!(
            linked.is_empty(),
            "a corrupt pairing store must fail closed to an empty linked set"
        );
        assert!(
            requesters_to_auto_accept(&[LINKED_BASE58.to_string()], &linked).is_empty(),
            "no auto-accept when the linked set is fail-closed empty"
        );
    }

    #[test]
    fn incoming_pending_requesters_filters_and_resolves_requester() {
        // Pending incoming only; `agentId` preferred, else `contact.requester`;
        // accepted requests and outgoing requests are ignored.
        let requests = serde_json::json!({
            "incoming": [
                { "agentId": "AAA", "status": "pending", "direction": "incoming",
                  "contact": { "requester": "AAA", "addressee": "me", "status": "pending" } },
                // agentId blank → fall back to contact.requester.
                { "agentId": "", "status": "pending", "direction": "incoming",
                  "contact": { "requester": "BBB", "addressee": "me", "status": "pending" } },
                // already accepted → skipped.
                { "agentId": "CCC", "status": "accepted", "direction": "incoming",
                  "contact": { "requester": "CCC", "addressee": "me", "status": "accepted" } },
            ],
            "outgoing": [
                // outgoing is never a request to accept.
                { "agentId": "DDD", "status": "pending", "direction": "outgoing",
                  "contact": { "requester": "me", "addressee": "DDD", "status": "pending" } },
            ],
        });
        assert_eq!(
            incoming_pending_requesters(&requests),
            vec!["AAA".to_string(), "BBB".to_string()]
        );

        // Missing/empty `incoming` → nothing, no panic.
        assert!(incoming_pending_requesters(&serde_json::json!({})).is_empty());
        assert!(incoming_pending_requesters(&serde_json::json!({ "incoming": [] })).is_empty());
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

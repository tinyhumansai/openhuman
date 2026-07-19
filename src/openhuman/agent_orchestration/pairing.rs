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

use chrono::{DateTime, Duration, Utc};

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::config::Config;
use crate::openhuman::orchestration::ingest::resolve_linked_id;
use crate::openhuman::tinyplace::ops::{global_state as tinyplace_state, map_err};

const LOG_TARGET: &str = "orchestration_pairing";

/// How long a local co-location handshake entry stays auto-acceptable. The CLI
/// writes its entry moments before it sends the contact request, so the window
/// is deliberately short — a long-lived entry would keep a since-departed agent
/// id auto-acceptable long after the CLI is gone.
fn local_handshake_ttl() -> Duration {
    Duration::hours(1)
}

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
    match client
        .http()
        .get_agent_auth::<Value>(&contact_path(agent_id, Some("status")), &[])
        .await
    {
        Ok(remote) => Ok(remote_status(&remote).unwrap_or_else(|| "none".to_string())),
        // A 404 from /contacts/{id}/status means "no contact relationship yet" —
        // the normal state before you've ever linked this agent. Treat it as
        // `none` (not an error) so `link_session` proceeds to send the request
        // instead of surfacing a false "not found" and aborting the first link.
        Err(e) if e.status() == Some(404) => {
            log::debug!(
                target: LOG_TARGET,
                "[orchestration_pairing] contact_status.none agent_id={agent_id} (404 = no relationship yet)"
            );
            Ok("none".to_string())
        }
        Err(e) => Err(map_err(e)),
    }
}

/// Whether the peer is an accepted tiny.place contact.
///
/// This is intentionally a live relay check rather than a read of the local
/// orchestration pairing store. An accepted contact is already a
/// user-consented recipient for an outbound message, even when it has not been
/// linked as an inbound orchestration session in the current workspace.
pub(crate) async fn is_accepted_contact(agent_id: &str) -> Result<bool, String> {
    let agent_id = normalize_agent_id(agent_id)?;
    Ok(contact_status(&agent_id).await? == "accepted")
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

/// One entry in the local co-location handshake file (`~/.openhuman/local-agents.json`).
/// A tiny.place CLI wrapper writes its own agent id + the OpenHuman owner it is
/// connecting to, moments before it sends its contact request. Because a contact
/// request carries NO owner declaration on the wire, this same-machine file is
/// how a freshly-connecting local agent proves "I am a co-located CLI that wants
/// THIS OpenHuman as my owner" — same-user filesystem access is the trust proof.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalAgentEntry {
    agent_id: String,
    /// The OpenHuman agent id this CLI declares as its owner. Auto-accept only
    /// fires when this matches OUR own id — so a CLI configured for a *different*
    /// local OpenHuman identity is never cross-accepted just for sharing a box.
    owner: String,
    /// RFC3339 write time. Required + TTL-bounded (fail-closed): a missing or
    /// stale timestamp is not trusted, so a long-dead entry can't linger as
    /// auto-acceptable.
    #[serde(default)]
    ts: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalAgentsFile {
    #[serde(default)]
    agents: Vec<LocalAgentEntry>,
}

/// Path of the local co-location handshake file, `~/.openhuman/local-agents.json`.
/// `None` when the home dir is unresolvable (never trust anything in that case).
fn local_agents_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".openhuman").join("local-agents.json"))
}

/// Fail-closed ownership/permission gate for a handshake path (issue #4777
/// review). Same-user filesystem access is the ONLY trust proof for
/// auto-accepting a co-located CLI's contact request, so a path another local
/// account could have written must never be trusted. On Unix, require the path
/// to be owned by the current effective uid AND not group/world-writable —
/// otherwise a permissive umask or shared home would let a different local user
/// plant a fresh owner-matching entry and get auto-accepted with no human click.
/// A missing/unreadable stat is untrusted (`false`). Non-Unix hosts trust the
/// path (the handshake is a same-machine dev-CLI feature; Windows ACLs are out
/// of scope) — every other trust source stays fail-closed regardless.
#[cfg(unix)]
fn path_is_privately_owned(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    // SAFETY: `geteuid` takes no arguments, mutates no state, and cannot fail.
    let euid = unsafe { libc::geteuid() };
    match std::fs::metadata(path) {
        Ok(meta) => meta.uid() == euid && (meta.mode() & 0o022) == 0,
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn path_is_privately_owned(_path: &Path) -> bool {
    true
}

/// Read + parse the local co-location handshake file. Missing file → empty (the
/// common case: no local CLI connecting). A parse/read error is logged and also
/// yields empty — fail-closed, trusts nothing. Cheap: local file read, no network.
fn load_local_agents() -> LocalAgentsFile {
    let Some(path) = local_agents_path() else {
        return LocalAgentsFile::default();
    };
    load_local_agents_from(&path)
}

/// Perm-gated read + parse of a handshake file at `path`. Split from
/// [`load_local_agents`] so the trust gate is unit-testable with a tempfile,
/// without depending on the real home dir.
fn load_local_agents_from(path: &Path) -> LocalAgentsFile {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return LocalAgentsFile::default()
        }
        Err(err) => {
            log::warn!(target: LOG_TARGET, "[orchestration_pairing] local_agents.read_failed: {err}");
            return LocalAgentsFile::default();
        }
    };
    // Trust proof (fail-closed): require the handshake file AND its parent
    // `.openhuman` dir to be privately owned by us and not writable by others.
    // On a multi-user host with a group/world-writable home (permissive umask,
    // shared box), another local account could otherwise inject a fresh
    // owner-matching entry and open a DM/orchestration channel to this brain
    // with no human approval. The parent-dir check also closes the symlink
    // vector (an attacker cannot plant a symlink in a dir they cannot write).
    let parent_ok = path.parent().map(path_is_privately_owned).unwrap_or(false);
    if !parent_ok || !path_is_privately_owned(path) {
        log::warn!(
            target: LOG_TARGET,
            "[orchestration_pairing] local_agents.untrusted_perms — file or parent dir not privately owned; refusing to trust handshake"
        );
        return LocalAgentsFile::default();
    }
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        log::warn!(target: LOG_TARGET, "[orchestration_pairing] local_agents.parse_failed: {e}");
        LocalAgentsFile::default()
    })
}

/// True when `ts` is a parseable RFC3339 stamp within the handshake TTL of `now`
/// (allowing minor clock skew into the future). A missing/malformed/expired stamp
/// is NOT fresh — fail-closed. Pure.
fn entry_is_fresh(ts: Option<&str>, now: DateTime<Utc>) -> bool {
    let Some(ts) = ts else {
        return false;
    };
    let Ok(written) = DateTime::parse_from_rfc3339(ts) else {
        return false;
    };
    let written = written.with_timezone(&Utc);
    let age = now.signed_duration_since(written);
    age >= -Duration::minutes(5) && age < local_handshake_ttl()
}

/// Reduce the local handshake file to the set of agent ids OpenHuman may
/// auto-accept: those declaring US (`own`) as owner AND still fresh. Pure — the
/// trust gate is unit-testable without any filesystem or network IO.
fn coload_trusted_ids(file: &LocalAgentsFile, own: &str, now: DateTime<Utc>) -> HashSet<String> {
    let own_set: HashSet<String> = std::iter::once(own.to_string()).collect();
    file.agents
        .iter()
        .filter(|entry| resolve_linked_id(&entry.owner, &own_set).is_some())
        .filter(|entry| entry_is_fresh(entry.ts.as_deref(), now))
        .map(|entry| entry.agent_id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect()
}

/// Poll the tiny.place incoming contact-request queue and auto-accept every
/// request whose requester is either (a) already a linked (paired) agent, or
/// (b) a co-located local CLI that named us as owner in the local handshake file
/// — and ONLY those. A request from any other agent is deliberately left
/// **pending** for the human to decide: generic auto-accept stays off, because
/// accepting a contact is a trust decision (the relay's own rule is "never
/// auto-accept"). The two exceptions are the user's *own* already-paired agents
/// and their *own* freshly-launched local CLI on this same machine.
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
/// Fail-closed on both sources: [`linked_agent_ids`] returns an **empty** set on
/// any pairing-store read error, and [`load_local_agents`] / [`coload_trusted_ids`]
/// yield **empty** on a missing/corrupt handshake file, an unresolvable owner id,
/// or a stale/undated entry — so a read failure or an unlocked-wallet miss
/// auto-accepts NOTHING (every request is left pending) rather than opening the
/// gate. Returns the number of requests accepted this pass.
pub async fn auto_accept_linked_contact_requests(config: &Config) -> Result<usize, String> {
    let linked = linked_agent_ids(&config.workspace_dir).await;
    // Candidate co-location entries (same-machine handshake file). Cheap local
    // read, no network, no wallet needed.
    let local = load_local_agents();
    // Nothing linked AND no local handshake candidates → nothing to match; skip
    // the round-trip AND avoid needing an unlocked wallet (fail-closed no-op).
    if linked.is_empty() && local.agents.is_empty() {
        return Ok(0);
    }
    let client = tinyplace_state().client().await?;
    // Resolve OUR own id so co-location entries can be owner-matched. Best-effort:
    // if the signer is unavailable we simply trust no local entries (linked-only).
    let trusted = match client.http().signer() {
        Some(signer) => coload_trusted_ids(&local, &signer.agent_id(), Utc::now()),
        None => {
            log::debug!(target: LOG_TARGET, "[orchestration_pairing] auto_accept.signer_unavailable");
            HashSet::new()
        }
    };
    // Union of the two trust sources. `requesters_to_auto_accept` canonicalizes a
    // requester's wire id against this set, so base64/base58 forms unify.
    let mut acceptable = linked.clone();
    acceptable.extend(trusted.iter().cloned());
    if acceptable.is_empty() {
        return Ok(0);
    }
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
    let to_accept = requesters_to_auto_accept(&incoming_pending_requesters(&requests), &acceptable);
    log::debug!(
        target: LOG_TARGET,
        "[orchestration_pairing] auto_accept.scan linked={} coload={} accept={}",
        linked.len(),
        trusted.len(),
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

    fn local_file(entries: &[(&str, &str, Option<&str>)]) -> LocalAgentsFile {
        LocalAgentsFile {
            agents: entries
                .iter()
                .map(|(agent_id, owner, ts)| LocalAgentEntry {
                    agent_id: agent_id.to_string(),
                    owner: owner.to_string(),
                    ts: ts.map(str::to_string),
                })
                .collect(),
        }
    }

    #[test]
    fn coload_trusts_fresh_entry_that_names_us_as_owner() {
        let now = DateTime::parse_from_rfc3339("2026-07-10T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        // A CLI (LINKED_BASE58) that named us (UNLINKED_BASE58) as owner, 1 min ago.
        let file = local_file(&[(LINKED_BASE58, UNLINKED_BASE58, Some("2026-07-10T11:59:00Z"))]);
        let trusted = coload_trusted_ids(&file, UNLINKED_BASE58, now);
        assert_eq!(
            trusted,
            [LINKED_BASE58.to_string()].into_iter().collect(),
            "a fresh entry naming us as owner is trusted"
        );
    }

    #[test]
    fn coload_owner_match_is_encoding_agnostic() {
        // The entry declares the owner in base64 while our own id is base58 — the
        // same identity. `resolve_linked_id` must unify them so the owner-match holds.
        let now = DateTime::parse_from_rfc3339("2026-07-10T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let file = local_file(&[(UNLINKED_BASE58, LINKED_BASE64, Some("2026-07-10T11:59:30Z"))]);
        let trusted = coload_trusted_ids(&file, LINKED_BASE58, now);
        assert_eq!(
            trusted,
            [UNLINKED_BASE58.to_string()].into_iter().collect(),
            "owner declared base64 must match our base58 own id"
        );
    }

    #[test]
    fn coload_rejects_entry_for_a_different_owner() {
        let now = DateTime::parse_from_rfc3339("2026-07-10T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        // Entry names LINKED_BASE58 as owner, but WE are UNLINKED_BASE58 → not ours.
        let file = local_file(&[(
            "SomeLocalCliAgentIdXXXXXXXXXXXXXXXXXXXXXXXXX",
            LINKED_BASE58,
            Some("2026-07-10T11:59:00Z"),
        )]);
        assert!(
            coload_trusted_ids(&file, UNLINKED_BASE58, now).is_empty(),
            "a CLI declaring a DIFFERENT local OpenHuman as owner is never trusted"
        );
    }

    #[test]
    fn coload_rejects_stale_or_undated_entries() {
        let now = DateTime::parse_from_rfc3339("2026-07-10T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        // Expired (2h old, TTL is 1h).
        let stale = local_file(&[(LINKED_BASE58, UNLINKED_BASE58, Some("2026-07-10T10:00:00Z"))]);
        assert!(
            coload_trusted_ids(&stale, UNLINKED_BASE58, now).is_empty(),
            "an entry past its TTL is not trusted"
        );
        // Missing timestamp → fail-closed.
        let undated = local_file(&[(LINKED_BASE58, UNLINKED_BASE58, None)]);
        assert!(
            coload_trusted_ids(&undated, UNLINKED_BASE58, now).is_empty(),
            "an entry with no timestamp is not trusted"
        );
    }

    #[test]
    fn entry_freshness_bounds() {
        let now = DateTime::parse_from_rfc3339("2026-07-10T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(entry_is_fresh(Some("2026-07-10T11:59:59Z"), now)); // just now
        assert!(entry_is_fresh(Some("2026-07-10T11:01:00Z"), now)); // 59 min → within TTL
        assert!(!entry_is_fresh(Some("2026-07-10T10:59:00Z"), now)); // 61 min → expired
        assert!(!entry_is_fresh(Some("2026-07-10T12:10:00Z"), now)); // 10 min future → skew reject
        assert!(!entry_is_fresh(Some("not-a-timestamp"), now)); // malformed
        assert!(!entry_is_fresh(None, now)); // absent
    }

    // ── Handshake-file trust gate (#4777 review — permission hardening) ───────

    /// A well-formed one-entry handshake JSON body naming us as owner.
    fn handshake_body() -> String {
        serde_json::json!({
            "agents": [{
                "agentId": LINKED_BASE58,
                "owner": UNLINKED_BASE58,
                "ts": "2026-07-10T11:59:00Z",
            }]
        })
        .to_string()
    }

    #[test]
    fn load_local_agents_from_missing_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("does-not-exist.json");
        assert!(
            load_local_agents_from(&path).agents.is_empty(),
            "a missing handshake file yields an empty (fail-closed) set"
        );
    }

    #[cfg(unix)]
    #[test]
    fn load_local_agents_from_trusts_privately_owned_file() {
        use std::os::unix::fs::PermissionsExt as _;
        let tmp = tempfile::tempdir().unwrap();
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = tmp.path().join("local-agents.json");
        std::fs::write(&path, handshake_body()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let file = load_local_agents_from(&path);
        assert_eq!(
            file.agents.len(),
            1,
            "a privately-owned handshake file is read"
        );
        assert_eq!(file.agents[0].agent_id, LINKED_BASE58);
    }

    #[cfg(unix)]
    #[test]
    fn load_local_agents_from_rejects_world_writable_file() {
        use std::os::unix::fs::PermissionsExt as _;
        let tmp = tempfile::tempdir().unwrap();
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = tmp.path().join("local-agents.json");
        std::fs::write(&path, handshake_body()).unwrap();
        // A group/world-writable file could have been forged by another local
        // user → fail closed and trust nothing.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(
            load_local_agents_from(&path).agents.is_empty(),
            "a world-writable handshake file is refused"
        );
    }

    #[cfg(unix)]
    #[test]
    fn load_local_agents_from_rejects_world_writable_dir() {
        use std::os::unix::fs::PermissionsExt as _;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("local-agents.json");
        std::fs::write(&path, handshake_body()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        // A world-writable parent dir lets another user swap the file (or plant a
        // symlink) → the file's own perms are not enough, so trust nothing.
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        let trusted_empty = load_local_agents_from(&path).agents.is_empty();
        // Restore private perms before the tempdir is cleaned up.
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o700)).ok();
        assert!(
            trusted_empty,
            "a world-writable parent dir makes the handshake untrusted"
        );
    }
}

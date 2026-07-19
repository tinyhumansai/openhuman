//! DM ingest: decrypt-once → classify → persist → acknowledge.
//!
//! Driven by the existing `DomainEvent::TinyPlaceStreamMessage` (the tinyplace
//! websocket recv loop), filtered to conversation/DM streams. Never logs message
//! bodies or seeds.

use std::collections::HashSet;
use std::path::Path;

use base64::Engine as _;

use crate::openhuman::config::Config;
use crate::openhuman::tinyplace::{acknowledge_message, decrypt_envelope};

use super::presence;
use super::store;
use super::types::{
    ChatKind, HarnessEventKind, OrchestrationMessage, OrchestrationSession, SessionEnvelopeV1,
    SessionEnvelopeV2,
};

const LOG: &str = "orchestration";

/// True when a tiny.place agent id `from` is one of the linked (paired) agents.
///
/// tiny.place identifies the *same* Ed25519 key two ways: the orchestration
/// pairing store keeps the **base58** Solana address (that's what the
/// contacts/directory API returns and what the pairing UI stores), while an
/// inbound `MessageEnvelope.from` carries the **base64** Ed25519 public key (the
/// relay's raw-key form). A plain string compare therefore treats a
/// legitimately-linked agent as unpaired and silently drops every DM it sends —
/// so the message never lands in the orchestration view. Compare by the decoded
/// 32-byte key so the two encodings unify. Fall back to the exact-string check
/// first (cheap, and covers ids that aren't 32-byte keys, e.g. a handle).
///
/// Shared with the contact-request auto-accept gate
/// (`agent_orchestration::pairing`): "is this id one of my linked agents?" is the
/// same trust question for an inbound DM and an inbound contact request, so both
/// resolve it through this single encoding-unifying matcher.
pub(crate) fn agent_id_in_linked_set(from: &str, linked: &HashSet<String>) -> bool {
    resolve_linked_id(from, linked).is_some()
}

/// Resolve `from` to the **canonical** linked-set id it matches — the exact
/// stored string if present, otherwise the stored id whose decoded 32-byte key
/// equals `from`'s (unifying the base58 pairing-store form and the base64 wire
/// form of one Ed25519 key). Returns `None` when `from` is not a linked agent.
///
/// A caller that then accepts/persists against the match MUST use this canonical
/// id, not the raw wire id: `PairingStore` dedupes records by exact-string
/// `agent_id`, so accepting under a base64 `from` while the linked record is
/// base58 would persist a *second* `Linked` record for the same identity — and
/// unlinking one encoding would leave the other still authorizing the peer.
pub(crate) fn resolve_linked_id(from: &str, linked: &HashSet<String>) -> Option<String> {
    if let Some(exact) = linked.get(from) {
        return Some(exact.clone());
    }
    let from_key = decode_agent_key(from)?;
    linked
        .iter()
        .find(|id| decode_agent_key(id) == Some(from_key))
        .cloned()
}

/// Decode a tiny.place agent identifier to its raw 32-byte Ed25519 public key,
/// accepting either the base64 (envelope `from`) or base58 (pairing store)
/// encoding. Returns `None` for anything that isn't a 32-byte key (handles,
/// malformed ids) so callers fall back to a string compare.
fn decode_agent_key(value: &str) -> Option<[u8; 32]> {
    // base64 (Ed25519 raw key) — the inbound `MessageEnvelope.from` form.
    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(value) {
        if let Ok(arr) = <[u8; 32]>::try_from(bytes.as_slice()) {
            return Some(arr);
        }
    }
    // base58 (Solana address) — the pairing-store form.
    if let Ok(bytes) = bs58::decode(value).into_vec() {
        if let Ok(arr) = <[u8; 32]>::try_from(bytes.as_slice()) {
            return Some(arr);
        }
    }
    None
}

/// A decrypted DM turned into the fields we persist. Pure result of
/// [`classify_message`] — no IO, so it is unit-testable without a live client.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ClassifiedMessage {
    chat_kind: ChatKind,
    session_id: String,
    role: String,
    source: String,
    label: Option<String>,
    workspace: Option<String>,
    /// The wire `line` (v1) / `event.seq` (v2). Retained for parity/debugging only
    /// — persistence deliberately IGNORES it and stamps a store ordinal (#4583).
    seq: i64,
    body: String,
    timestamp: String,
    // ── v2 event shape (all `None`/false for v1 + Master) ────────────────────
    /// v2 `event.kind`; drives the per-message render branch (Phase 2).
    event_kind: Option<String>,
    /// v2 tool identity for `tool_call` / `approval_request`.
    tool_name: Option<String>,
    /// v2 `call_id` correlating `tool_result` → `tool_call`.
    call_id: Option<String>,
    /// v2 `tool_result` outcome (`None` for rows that are not `tool_result`).
    ok: Option<bool>,
    is_error: Option<bool>,
    exit_code: Option<i64>,
    /// v2 `status.state` (or a derived state for approval/lifecycle/error) written
    /// onto the session row so `derive_status` reads a real run-state.
    status_state: Option<String>,
    /// v2 `status.detail` → the session's `current_detail` (roster task line).
    status_detail: Option<String>,
    /// v2 `status.active_call_id` → the session's `active_call_id`.
    active_call_id: Option<String>,
    /// Whether this event advances the monotonic wake ordinal. Content events
    /// (prompts/messages/thinking/tool_call/tool_result/approval/error) do; pure
    /// session-state events (`status`/`lifecycle`/`unknown`) do NOT, so a status
    /// ping never spuriously wakes the front-end graph.
    advances_seq: bool,
    /// True only for an authoritative `status` snapshot, which OWNS the session's
    /// run-state columns and must be able to CLEAR `current_detail`/`active_call_id`
    /// (e.g. `running_tool` → `idle`). Content events leave this false so the
    /// COALESCE upsert preserves the last status instead of wiping it.
    authoritative_status: bool,
    // ── session_info enrichment (only a `session_info` event sets these) ──────
    /// `session_info.title` → the session row's `title`.
    title: Option<String>,
    /// `session_info.model` (or `event.model` fallback) → the session row's `model`.
    model: Option<String>,
    /// `session_info.handle` → the session row's `handle`.
    handle: Option<String>,
    /// `session_info.repo` → the session row's `repo`.
    repo: Option<String>,
    /// `session_info.branch` → the session row's `branch`.
    branch: Option<String>,
    /// `session_info.capabilities` → the session row's `capabilities`.
    capabilities: Vec<String>,
}

/// True for streams that carry ciphertext DM envelopes worth ingesting.
fn is_dm_stream(kind: &str, stream_id: &str) -> bool {
    kind.eq_ignore_ascii_case("conversation")
        || kind.eq_ignore_ascii_case("dm")
        || stream_id.starts_with("conversation:")
}

/// True when a `decrypt_envelope` error is a permanent Signal-layer decryption
/// failure (no session, bad MAC, malformed ciphertext) rather than a transient
/// one (key-bundle fetch, network, store IO). `decrypt_envelope` prefixes every
/// Signal decrypt error with `"decryption failed: "`, so that marker is the
/// discriminator. Permanent failures are dead-lettered so a single unreadable
/// envelope can't poison the drain loop forever. Pure.
fn is_unrecoverable_decrypt_error(err: &str) -> bool {
    err.contains("decryption failed")
}

/// Stable-sort a batch of envelopes so session-establishing PREKEY_BUNDLE
/// messages are processed before CIPHERTEXT ones. Pure.
fn order_prekey_bundles_first(messages: &mut [tinyplace::types::MessageEnvelope]) {
    messages.sort_by_key(|m| {
        u8::from(m.envelope_type != tinyplace::signal::session::TYPE_PREKEY_BUNDLE)
    });
}

/// Classify a decrypted DM into the fields we persist. Version-dispatched: try a
/// v2 harness envelope first, then a v1 envelope, else the peer's Master window.
/// Both envelope versions discriminate on `envelope_version`, so the order is
/// safe (a v1 body never matches v2 and vice-versa); this is the v1↔v2
/// coexistence seam — both persist into the same session model. Pure.
fn classify_message(plaintext: String, fallback_timestamp: &str) -> ClassifiedMessage {
    if let Some(env) = SessionEnvelopeV2::parse(&plaintext) {
        return classify_v2(env, fallback_timestamp);
    }
    if let Some(env) = SessionEnvelopeV1::parse(&plaintext) {
        return classify_v1(env, fallback_timestamp);
    }
    // Not a harness envelope → a plain DM in the peer's Master window.
    //
    // This path only ever runs on an INBOUND, decrypted DM (the counterpart is
    // the sender — see `ingest_one`), so the author is the PEER, never us. Stamp
    // `"peer"` rather than `"user"`: the transcript treats `you`/`owner`/`user`
    // as owner-authored and right-aligns them, so a peer's incoming Master DM was
    // rendering on the owner's side (both halves of a two-way DM stacked right).
    // `"peer"` is not owner-authored → it left-aligns as the counterpart. Our own
    // outgoing Master message is persisted separately as `"owner"` and stays right.
    ClassifiedMessage {
        chat_kind: ChatKind::Master,
        session_id: "master".to_string(),
        role: "peer".to_string(),
        source: String::new(),
        label: None,
        workspace: None,
        seq: 0,
        body: plaintext,
        timestamp: fallback_timestamp.to_string(),
        event_kind: None,
        tool_name: None,
        call_id: None,
        ok: None,
        is_error: None,
        exit_code: None,
        status_state: None,
        status_detail: None,
        active_call_id: None,
        advances_seq: true,
        authoritative_status: false,
        // session_info enrichment — only a v2 `session_info` event populates these.
        title: None,
        model: None,
        handle: None,
        repo: None,
        branch: None,
        capabilities: Vec::new(),
    }
}

/// Classify a v1 harness envelope — the original per-session mapping, unchanged.
fn classify_v1(env: SessionEnvelopeV1, fallback_timestamp: &str) -> ClassifiedMessage {
    // Compute the session key while `env` is still fully intact (before any
    // field moves below), since `session_key` borrows `&env`.
    let session_id = env.session_key();
    let label = (env.scope.scope_type == "folder").then(|| env.scope.key.clone());
    let workspace = (!env.scope.cwd.is_empty()).then(|| env.scope.cwd.clone());
    let timestamp = if env.message.timestamp.is_empty() {
        fallback_timestamp.to_string()
    } else {
        env.message.timestamp
    };
    ClassifiedMessage {
        chat_kind: ChatKind::Session,
        // Key on the single per-pair session id (the shared `wrapper_session_id`
        // both peers put on every message for a thread), so a reply threads back
        // into the same session. Falls back to `harness_session_id` for a legacy
        // envelope with no per-pair id. See `SessionEnvelopeV1::session_key`.
        session_id,
        role: env.message.role,
        source: env.harness.provider,
        label,
        workspace,
        seq: env.message.line,
        body: env.message.text,
        timestamp,
        event_kind: None,
        tool_name: None,
        call_id: None,
        ok: None,
        is_error: None,
        exit_code: None,
        status_state: None,
        status_detail: None,
        active_call_id: None,
        advances_seq: true,
        authoritative_status: false,
        // session_info enrichment — only a v2 `session_info` event populates these.
        title: None,
        model: None,
        handle: None,
        repo: None,
        branch: None,
        capabilities: Vec::new(),
    }
}

/// Classify a v2 harness envelope. Switches on `event.kind`, mapping each to the
/// persisted fields (per the plan's mapping table). Content events become thread
/// messages that advance the wake ordinal; `status`/`lifecycle`/`unknown` are
/// session-state-only (they still persist a row for id-dedupe + ack, but do NOT
/// advance the wake ordinal). Pure.
fn classify_v2(env: SessionEnvelopeV2, fallback_timestamp: &str) -> ClassifiedMessage {
    let session_id = env.session_key();
    let label = (env.scope.scope_type == "folder").then(|| env.scope.key.clone());
    let workspace = (!env.scope.cwd.is_empty()).then(|| env.scope.cwd.clone());
    let source = env.harness.provider.clone();
    let timestamp = if env.event.ts.is_empty() {
        fallback_timestamp.to_string()
    } else {
        env.event.ts.clone()
    };
    let kind_str = env.event.kind.clone();
    let wire_role = env.event.role.clone();
    let seq = env.event.seq;
    // `session_info.model` is optional; fall back to the frame's `event.model`.
    let event_model = env.event.model.clone();

    // Per-kind body + tool/status fields + wake disposition.
    let mut b = V2Body::default();
    match env.event.decoded() {
        HarnessEventKind::SessionInfo(p) => {
            // Session intro/announce: enrichment, not a chat bubble. Persist a
            // seq=0 row (id-dedupe + ack) that does NOT advance the wake ordinal —
            // same disposition as status/lifecycle — and fold the payload into the
            // session record fields. `title` doubles as the row body so the
            // session_info message carries a human-readable trace of the intro.
            b.body = p.title.clone().unwrap_or_default();
            b.advances_seq = false;
            b.title = p.title;
            b.model = p.model.or(event_model);
            b.handle = p.handle;
            b.repo = p.repo;
            b.branch = p.branch;
            b.capabilities = p.capabilities;
        }
        HarnessEventKind::UserPrompt(p) => {
            b.body = p.text;
            b.default_role = "owner";
        }
        HarnessEventKind::AgentMessage(p) => {
            b.body = p.text;
        }
        HarnessEventKind::AgentThinking(p) => {
            b.body = p.text;
        }
        HarnessEventKind::ToolCall(p) => {
            b.body = p.display;
            b.tool_name = non_empty(p.tool_name);
            b.call_id = non_empty(p.call_id);
        }
        HarnessEventKind::ToolResult(p) => {
            b.body = p.output;
            b.call_id = non_empty(p.call_id);
            // Carry the outcome so the renderer can distinguish a failed run.
            b.ok = p.ok;
            b.is_error = Some(p.is_error);
            b.exit_code = p.exit_code;
        }
        HarnessEventKind::ApprovalRequest(p) => {
            b.body = p.display;
            b.tool_name = non_empty(p.tool_name);
            b.call_id = p.call_id.and_then(non_empty);
            // Drive the roster dot to waiting-approval.
            b.status_state = Some("waiting_approval".to_string());
        }
        HarnessEventKind::Status(p) => {
            b.body = p.detail.clone();
            b.status_state = non_empty(p.state);
            b.status_detail = non_empty(p.detail);
            b.active_call_id = p.active_call_id.and_then(non_empty);
            b.advances_seq = false;
            // Authoritative run-state snapshot: it may CLEAR detail/active_call_id
            // (running_tool → idle), so persistence overwrites rather than COALESCEs.
            b.authoritative_status = true;
        }
        HarnessEventKind::Lifecycle(p) => {
            b.body = p.phase.clone();
            // A session_end lifecycle marks the instance stopped; other phases
            // carry no run-state.
            if p.phase == "session_end" {
                b.status_state = Some("stopped".to_string());
            }
            b.advances_seq = false;
        }
        HarnessEventKind::Error(p) => {
            b.body = p.message;
            b.status_state = Some("errored".to_string());
        }
        HarnessEventKind::Unknown(p) => {
            // Preserve the raw payload as the body so nothing is silently lost.
            b.body = serde_json::to_string(&p.raw).unwrap_or_default();
            b.advances_seq = false;
            // Persist as the literal `unknown` (not the raw wire kind) so the store
            // readers keep a forward/garbled event out of the thread + unread count.
            b.kind_override = Some("unknown");
        }
    }

    let role = if !wire_role.is_empty() {
        wire_role
    } else {
        b.default_role.to_string()
    };

    ClassifiedMessage {
        chat_kind: ChatKind::Session,
        session_id,
        role,
        source,
        label,
        workspace,
        seq,
        body: b.body,
        timestamp,
        event_kind: Some(b.kind_override.map(str::to_string).unwrap_or(kind_str)),
        tool_name: b.tool_name,
        call_id: b.call_id,
        ok: b.ok,
        is_error: b.is_error,
        exit_code: b.exit_code,
        status_state: b.status_state,
        status_detail: b.status_detail,
        active_call_id: b.active_call_id,
        advances_seq: b.advances_seq,
        authoritative_status: b.authoritative_status,
        title: b.title,
        model: b.model,
        handle: b.handle,
        repo: b.repo,
        branch: b.branch,
        capabilities: b.capabilities,
    }
}

/// Per-kind accumulator for [`classify_v2`], so the big match stays a set of small
/// assignments with sensible defaults (content event, `agent` role).
struct V2Body {
    body: String,
    default_role: &'static str,
    tool_name: Option<String>,
    call_id: Option<String>,
    ok: Option<bool>,
    is_error: Option<bool>,
    exit_code: Option<i64>,
    status_state: Option<String>,
    status_detail: Option<String>,
    active_call_id: Option<String>,
    advances_seq: bool,
    authoritative_status: bool,
    /// Overrides the persisted `event_kind` for a forward/garbled kind that
    /// `decoded()` folded to `Unknown`: stored as the literal `"unknown"` so the
    /// store readers keep it out of the thread instead of leaking the raw kind.
    kind_override: Option<&'static str>,
    // ── session_info enrichment (only the `SessionInfo` arm sets these) ───────
    title: Option<String>,
    model: Option<String>,
    handle: Option<String>,
    repo: Option<String>,
    branch: Option<String>,
    capabilities: Vec<String>,
}

impl Default for V2Body {
    fn default() -> Self {
        V2Body {
            body: String::new(),
            default_role: "agent",
            tool_name: None,
            call_id: None,
            ok: None,
            is_error: None,
            exit_code: None,
            status_state: None,
            status_detail: None,
            active_call_id: None,
            advances_seq: true,
            authoritative_status: false,
            kind_override: None,
            title: None,
            model: None,
            handle: None,
            repo: None,
            branch: None,
            capabilities: Vec::new(),
        }
    }
}

/// `Some(s)` when non-empty, else `None` — so blank wire strings persist as NULL.
fn non_empty(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

/// Persist a classified message + its session row. Idempotent by `msg_id`;
/// returns true if a new message row landed. Testable with a tempdir DB.
/// Persist a classified message. Returns `Some(ingest_seq)` when a new row
/// landed (the store-assigned per-session ordinal), or `None` when the message
/// was a duplicate and nothing was written. The seq is the idempotency key the
/// shadow cloud push (`cloud::push_event`) uploads with.
fn persist_message(
    workspace_dir: &Path,
    msg_id: &str,
    agent_id: &str,
    classified: &ClassifiedMessage,
    now: &str,
) -> Result<Option<i64>, String> {
    store::with_connection(workspace_dir, |c| {
        // Wake idempotence keys on a per-session `seq` being monotonic, but the
        // harness `message.line` we classify into `seq` is NOT reliable: a wrapped
        // Claude harness stamps `line = 0` on every DM, and a peer reusing one
        // `wrapper_session_id` across harness sessions can reset it. Under the
        // shared per-pair session key that collapses every message into one
        // session whose `last_seq`/wake cursor then pins at 0, so after the first
        // message the graph is skipped and the DM is silently dropped (#4583).
        //
        // Fix: ignore the wire `line` for ordering and stamp a store-assigned,
        // strictly-increasing per-(agent, session) ingest ordinal. Messages are
        // append-only and deduped-by-id upstream, so `MAX(seq)+1` is monotonic and
        // every genuinely-new DM advances `last_seq` past the cursor → wakes the graph.
        //
        // Allocate the ordinal and write both rows in one IMMEDIATE txn so a
        // concurrent writer on the same session (the drain here vs the graph's
        // `send_dm` reply persist) can't read the same `MAX(seq)` and duplicate it.
        store::in_immediate_txn(c, |c| {
            // Content events advance the monotonic wake ordinal (the #4583 fix);
            // pure session-state events (status/lifecycle/unknown) persist a row
            // for id-dedupe + ack but stamp `seq = 0` so they do NOT advance
            // `last_seq` and therefore never spuriously wake the front-end graph.
            // (`upsert_session` clamps `last_seq` with `MAX(...)`, so a 0 here only
            // refreshes `last_message_at` + the status columns.)
            let ingest_seq = if classified.advances_seq {
                store::next_session_seq(c, agent_id, &classified.session_id)?
            } else {
                0
            };
            store::upsert_session(
                c,
                &OrchestrationSession {
                    session_id: classified.session_id.clone(),
                    agent_id: agent_id.to_string(),
                    source: classified.source.clone(),
                    label: classified.label.clone(),
                    workspace: classified.workspace.clone(),
                    last_seq: ingest_seq,
                    created_at: now.to_string(),
                    last_message_at: classified.timestamp.clone(),
                    status_state: classified.status_state.clone(),
                    current_detail: classified.status_detail.clone(),
                    active_call_id: classified.active_call_id.clone(),
                    title: classified.title.clone(),
                    model: classified.model.clone(),
                    handle: classified.handle.clone(),
                    repo: classified.repo.clone(),
                    branch: classified.branch.clone(),
                    capabilities: classified.capabilities.clone(),
                },
            )?;
            // An authoritative `status` snapshot OWNS the run-state columns and may
            // CLEAR them (running_tool → idle); the COALESCE upsert above can't, so
            // overwrite them directly. Content events skip this and keep COALESCE.
            if classified.authoritative_status {
                store::apply_run_state(
                    c,
                    agent_id,
                    &classified.session_id,
                    classified.status_state.as_deref(),
                    classified.status_detail.as_deref(),
                    classified.active_call_id.as_deref(),
                )?;
            }
            let landed = store::insert_message(
                c,
                &OrchestrationMessage {
                    id: msg_id.to_string(),
                    agent_id: agent_id.to_string(),
                    session_id: classified.session_id.clone(),
                    chat_kind: classified.chat_kind,
                    role: classified.role.clone(),
                    body: classified.body.clone(),
                    timestamp: classified.timestamp.clone(),
                    seq: ingest_seq,
                    event_kind: classified.event_kind.clone(),
                    tool_name: classified.tool_name.clone(),
                    call_id: classified.call_id.clone(),
                    ok: classified.ok,
                    is_error: classified.is_error,
                    exit_code: classified.exit_code,
                },
            )?;
            // An `error` event also records the short cause on the status surface
            // (`orchestration.status.last_error`). Body is a harness error message,
            // never a response body — safe to store (workspace-internal DB).
            if landed && classified.event_kind.as_deref() == Some("error") {
                store::kv_set(c, "orchestration:last_error", &classified.body)?;
            }
            Ok(landed.then_some(ingest_seq))
        })
    })
    .map_err(|e| format!("persist: {e}"))
}

/// Forward a freshly-landed event to the hosted brain
/// (`POST /orchestration/v1/events`), which runs the wake cycle server-side.
/// Builds the sanitized wire envelope (the security allowlist lives in
/// [`super::wire`]) and spawns the upload so ingest never blocks on the network.
/// Best-effort/fire-and-forget: a push failure (or offline) is logged and
/// dropped — the render cache still holds the row and the first-login migration
/// replay backfills anything missed while offline.
fn forward_event(
    config: &Config,
    agent_id: &str,
    ingest_seq: i64,
    classified: &ClassifiedMessage,
    now: &str,
) {
    // Content events carry the real per-session ordinal; a status/lifecycle
    // event stamps seq 0 and would collide on the backend's idempotency key, so
    // only forward seq-advancing events.
    if ingest_seq <= 0 {
        return;
    }
    let ts = super::wire::parse_ts_ms(&classified.timestamp)
        .or_else(|| super::wire::parse_ts_ms(now))
        .unwrap_or(0);
    let kind = classified
        .event_kind
        .as_deref()
        .unwrap_or_else(|| classified.chat_kind.as_str());
    let envelope = super::wire::OrchestrationEventEnvelopeWire::build(
        agent_id,
        &classified.session_id,
        ingest_seq,
        &classified.role,
        // For an inbound DM the sender is the counterpart agent itself.
        agent_id,
        &classified.body,
        ts,
        kind,
    );
    let config = config.clone();
    tokio::spawn(async move {
        match super::cloud::push_event(&config, &envelope).await {
            Ok(cycle_id) => {
                // Device-authoritative origin: record the cycle → counterpart we
                // forwarded, so the local-execution trust gate can resolve this
                // cycle's origin without trusting the backend (see `exec_gate`).
                if let Some(cid) = cycle_id {
                    super::exec_gate::record_cycle_origin(
                        &cid,
                        &envelope.counterpart_agent_id,
                        &envelope.session_id,
                    );
                }
            }
            Err(e) => log::warn!(target: LOG, "[orchestration] cloud.shadow_push_failed: {e}"),
        }
    });
}

/// Entry point from the bus subscriber. Cheap no-op when orchestration is
/// disabled or the stream is not a DM stream.
pub async fn ingest_stream_message(
    config: &Config,
    kind: &str,
    stream_id: &str,
    raw: &serde_json::Value,
) {
    if !config.orchestration.enabled {
        return;
    }
    if !is_dm_stream(kind, stream_id) {
        return;
    }
    let envelope: tinyplace::types::MessageEnvelope = match serde_json::from_value(raw.clone()) {
        Ok(env) => env,
        Err(e) => {
            log::debug!(target: LOG, "[orchestration] ingest.skip stream={stream_id} not-an-envelope err={e}");
            return;
        }
    };
    if let Err(e) = ingest_one(config, envelope).await {
        log::warn!(target: LOG, "[orchestration] ingest.error stream={stream_id}: {e}");
    }
}

async fn ingest_one(
    config: &Config,
    envelope: tinyplace::types::MessageEnvelope,
) -> Result<(), String> {
    let msg_id = envelope.id.clone();
    let agent_id = envelope.from.clone();
    log::debug!(target: LOG, "[orchestration] ingest.entry id={msg_id} from={agent_id}");
    let workspace_dir = config.workspace_dir.clone();

    // 0. Sender gate: only ingest DMs from linked (accepted) pairing agents —
    //    i.e. wrapped Codex/Claude sessions. Decrypting advances the Signal
    //    ratchet, so an unpaired sender's DM (an ordinary human message) must
    //    never be decrypted or consumed here; it stays readable by the existing
    //    Messaging UI via messages.list / signal.decryptMessage.
    let linked =
        crate::openhuman::agent_orchestration::pairing::linked_agent_ids(&workspace_dir).await;
    if !agent_id_in_linked_set(&agent_id, &linked) {
        log::debug!(
            target: LOG,
            "[orchestration] ingest.skip_unpaired from={agent_id} linked_count={}",
            linked.len()
        );
        return Ok(());
    }

    // 1. Dedupe BEFORE decrypt — protects the non-idempotent Signal ratchet.
    let already = store::with_connection(&workspace_dir, |c| store::message_exists(c, &msg_id))
        .map_err(|e| format!("store lookup: {e}"))?;
    if already {
        // The row already exists but a prior run may have crashed (or the relay
        // ack failed) after persist. Retry the ack so the relay copy is
        // consumed; never re-decrypt or re-publish.
        log::debug!(target: LOG, "[orchestration] ingest.dedupe id={msg_id}");
        if let Err(e) = acknowledge_message(&msg_id).await {
            log::warn!(target: LOG, "[orchestration] ingest.ack_failed_dedupe id={msg_id}: {e}");
        }
        return Ok(());
    }

    // 2. Decrypt exactly once, then classify + persist.
    //
    // A Signal-layer decryption failure ("No session", bad MAC, malformed body)
    // is PERMANENT for this envelope: the ratchet state needed to read it is
    // gone or was never established (e.g. a CIPHERTEXT whose establishing
    // PREKEY_BUNDLE was lost, or a session reset on our side). Because we only
    // acknowledge on success (stage 3), leaving such an envelope in the mailbox
    // makes every subsequent drain re-fetch, re-attempt, and re-log it forever —
    // a poison message that also grows the mailbox unboundedly. So dead-letter
    // it: acknowledge (consume) once and move on. Transient errors (bundle
    // fetch, network, store IO) are NOT swallowed — they are returned so the
    // envelope is retried on the next drain.
    let plaintext = match decrypt_envelope(&envelope).await {
        Ok(plaintext) => plaintext,
        Err(e) if is_unrecoverable_decrypt_error(&e) => {
            log::warn!(
                target: LOG,
                "[orchestration] ingest.drop_undecryptable from={agent_id} id={msg_id}: {e}"
            );
            if let Err(ack) = acknowledge_message(&msg_id).await {
                log::warn!(target: LOG, "[orchestration] ingest.ack_failed_drop id={msg_id}: {ack}");
            }
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    // A decryptable inbound envelope proves the peer is reachable right now —
    // feed the live presence map (drives the online/offline indicator).
    presence::mark_seen(&agent_id);
    let classified = classify_message(plaintext, &envelope.timestamp);
    let now = chrono::Utc::now().to_rfc3339();
    let landed = persist_message(&workspace_dir, &msg_id, &agent_id, &classified, &now)?;

    // 3. Acknowledge (consume once) + fan out for stages 4/7.
    if let Some(ingest_seq) = landed {
        if let Err(e) = acknowledge_message(&msg_id).await {
            log::warn!(target: LOG, "[orchestration] ingest.ack_failed id={msg_id}: {e}");
        }

        // Forward the sanitized event to the hosted brain, which runs the wake
        // cycle and replies via socket effects. The device no longer wakes a
        // local graph.
        forward_event(config, &agent_id, ingest_seq, &classified, &now);

        // Record a compact device world-observation for the hosted subconscious
        // tier; the periodic uploader batches these to the world-diff route,
        // whose receipt schedules a steering tick. Never carries the body — only
        // a bounded derived note (see `world_model`).
        let obs_ts = super::wire::parse_ts_ms(&classified.timestamp)
            .or_else(|| super::wire::parse_ts_ms(&now))
            .unwrap_or(0);
        let obs_note = super::world_model::observe_ingest_note(
            &classified.session_id,
            &agent_id,
            classified
                .event_kind
                .as_deref()
                .unwrap_or_else(|| classified.chat_kind.as_str()),
            &classified.body,
        );
        if let Err(e) = store::with_connection(&workspace_dir, |c| {
            store::append_world_obs(c, &classified.session_id, &obs_note, obs_ts)
        }) {
            log::warn!(target: LOG, "[orchestration] world_obs.append_failed id={msg_id}: {e}");
        }

        // Live-UI nudge: fan the persisted message to the renderer socket so the
        // affected chat targeted-refetches. Reasoning/reply is the hosted brain's
        // job now — this is presentation only, no wake scheduling.
        super::bus::notify_orchestration_message(
            &agent_id,
            &classified.session_id,
            classified.chat_kind.as_str(),
        );
    }
    log::debug!(
        target: LOG,
        "[orchestration] ingest.exit id={msg_id} landed={}",
        landed.is_some()
    );
    Ok(())
}

/// Poll the relay mailbox once and run every delivered envelope through the
/// ingest pipeline.
///
/// The relay delivers DMs to `/messages` (poll-only) and — unlike inbox items —
/// never publishes them to the `/inbox/stream` WebSocket, so a poller is the
/// only way orchestration learns about inbound DMs. Envelopes from senders that
/// are not orchestration-linked are skipped by [`ingest_one`] WITHOUT being
/// decrypted or acknowledged, so they stay in the mailbox for the Messaging UI.
///
/// Returns the number of envelopes examined this pass. Best-effort per envelope:
/// a decrypt/persist failure on one is logged and does not abort the batch.
pub async fn drain_mailbox_once(config: &Config) -> Result<usize, String> {
    if !config.orchestration.enabled {
        return Ok(0);
    }
    let client = crate::openhuman::tinyplace::ops::global_state()
        .client()
        .await?;
    let signer = client
        .http()
        .signer()
        .ok_or_else(|| "no signer configured".to_string())?;
    let agent_id = signer.agent_id();
    let resp = client
        .messages
        .list(&agent_id, Some(100))
        .await
        .map_err(|e| format!("messages.list: {e}"))?;
    let mut messages = resp.messages;
    let count = messages.len();
    if count > 0 {
        log::debug!(target: LOG, "[orchestration] drain.fetched count={count}");
    }
    // Process session-establishing PREKEY_BUNDLE envelopes before CIPHERTEXT
    // ones: relay list order is not guaranteed chronological, so a first-contact
    // batch could otherwise hand a CIPHERTEXT to `ingest_one` before the
    // PREKEY_BUNDLE that sets up its Signal session, needlessly failing (and,
    // now, dead-lettering) a message that was about to become decryptable. The
    // sort is stable, so same-type envelopes keep their delivered order.
    order_prekey_bundles_first(&mut messages);
    for envelope in messages {
        if let Err(e) = ingest_one(config, envelope).await {
            log::warn!(target: LOG, "[orchestration] drain.ingest_error: {e}");
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENVELOPE: &str = r#"{
        "envelope_version": "tinyplace.harness.session.v1",
        "version": 1,
        "scope": { "type": "folder", "key": "my-repo", "cwd": "/w",
                   "wrapper_session_id": "w1", "harness_session_id": "h1" },
        "harness": { "provider": "codex", "command": "codex", "argv": [] },
        "message": { "id": "m1", "line": 7, "role": "user", "text": "hello",
                     "timestamp": "2026-07-02T01:00:00Z" },
        "source": { "path": "p", "record_type": "user" }
    }"#;

    #[test]
    fn dm_stream_filter() {
        assert!(is_dm_stream("conversation", "conversation:abc"));
        assert!(is_dm_stream("DM", "x"));
        assert!(is_dm_stream("other", "conversation:abc"));
        assert!(!is_dm_stream("inbox", "inbox"));
    }

    #[test]
    fn linked_gate_unifies_base58_and_base64_of_same_key() {
        // Real pair observed in the wild: the pairing store holds the base58
        // Solana address; the inbound `MessageEnvelope.from` holds the base64
        // Ed25519 public key. Both are the same 32-byte key, so a linked agent
        // must be recognised regardless of which encoding arrives.
        let base58 = "7jr5FKYETssD6T1MCzsR4aT4dnjjyJCE2SANYYX1R5vm";
        let base64 = "ZCAAuA+2GVoRrT08Gt8JUVnxnISTelSxnDuyScze334=";
        assert_eq!(
            decode_agent_key(base58),
            decode_agent_key(base64),
            "base58 and base64 forms must decode to the same key"
        );

        // Pairing store keeps the base58 address; the DM arrives base64.
        let linked: HashSet<String> = [base58.to_string()].into_iter().collect();
        assert!(
            agent_id_in_linked_set(base64, &linked),
            "a base64 `from` must match a base58-stored linked agent"
        );

        // Exact-string match still works (both base58), and the reverse too.
        assert!(agent_id_in_linked_set(base58, &linked));
        let linked_b64: HashSet<String> = [base64.to_string()].into_iter().collect();
        assert!(agent_id_in_linked_set(base58, &linked_b64));

        // An unrelated key is still rejected.
        let other = "De6RHrMj6eDqX1WBTXk11sks4WXHMaqEX9A6oQ3ZEmsg";
        assert!(!agent_id_in_linked_set(other, &linked));

        // `resolve_linked_id` unifies encodings AND returns the CANONICAL stored
        // id (base58), never the raw base64 wire form — so a caller accepting
        // against the match reuses the one linked record instead of duplicating it.
        assert_eq!(
            resolve_linked_id(base64, &linked).as_deref(),
            Some(base58),
            "a base64 `from` must canonicalize to the stored base58 id"
        );
        assert_eq!(resolve_linked_id(base58, &linked).as_deref(), Some(base58));
        assert_eq!(resolve_linked_id(other, &linked), None);
    }

    #[test]
    fn linked_gate_falls_back_to_string_for_non_key_ids() {
        // A handle (not a 32-byte key) can only match by exact string.
        let linked: HashSet<String> = ["@codex-handle".to_string()].into_iter().collect();
        assert!(agent_id_in_linked_set("@codex-handle", &linked));
        assert!(!agent_id_in_linked_set("@other-handle", &linked));
        assert!(decode_agent_key("@codex-handle").is_none());
    }

    #[tokio::test]
    async fn drain_is_a_noop_when_orchestration_disabled() {
        // Guard short-circuits before touching the tiny.place client, so this
        // exercises the early return without any wallet/network.
        let mut config = Config::default();
        config.orchestration.enabled = false;
        assert_eq!(drain_mailbox_once(&config).await, Ok(0));
    }

    #[test]
    fn classifies_harness_envelope_as_session() {
        let c = classify_message(ENVELOPE.to_string(), "2026-07-02T09:00:00Z");
        assert_eq!(c.chat_kind, ChatKind::Session);
        assert_eq!(c.session_id, "w1"); // keyed on the shared per-pair wrapper_session_id
        assert_eq!(c.role, "user");
        assert_eq!(c.source, "codex");
        assert_eq!(c.label.as_deref(), Some("my-repo")); // folder scope → label
        assert_eq!(c.workspace.as_deref(), Some("/w"));
        assert_eq!(c.seq, 7);
        assert_eq!(c.body, "hello");
        assert_eq!(c.timestamp, "2026-07-02T01:00:00Z"); // envelope ts wins
    }

    #[test]
    fn classifies_plain_dm_as_master_with_fallback_timestamp() {
        let c = classify_message("just chatting".to_string(), "2026-07-02T09:00:00Z");
        assert_eq!(c.chat_kind, ChatKind::Master);
        assert_eq!(c.session_id, "master");
        // Inbound Master DMs are peer-authored → NOT an owner-authored role
        // (`you`/`owner`/`user`), so the transcript left-aligns them.
        assert_eq!(c.role, "peer");
        assert!(!["you", "owner", "user"].contains(&c.role.as_str()));
        assert!(c.label.is_none());
        assert_eq!(c.seq, 0);
        assert_eq!(c.body, "just chatting");
        assert_eq!(c.timestamp, "2026-07-02T09:00:00Z"); // fallback used
    }

    /// A v2 wire envelope for `kind`/`payload`, keyed on `wrapper` session id.
    fn v2_env(kind: &str, payload: &str, wrapper: &str, role: &str) -> String {
        format!(
            r#"{{
                "envelope_version": "tinyplace.harness.session.v2",
                "version": 2,
                "scope": {{ "type": "folder", "key": "my-repo", "cwd": "/w",
                           "wrapper_session_id": "{wrapper}", "harness_session_id": "h2" }},
                "harness": {{ "provider": "claude", "command": "claude", "argv": [] }},
                "event": {{ "id": "e1", "seq": 9, "ts": "2026-07-05T01:00:00Z",
                           "role": "{role}", "kind": "{kind}", "payload": {payload} }},
                "source": {{ "path": "p", "record_type": "assistant" }}
            }}"#
        )
    }

    #[test]
    fn classifies_v2_content_events_with_kind_and_tool_fields() {
        // agent_message → session message, role from wire, event_kind set.
        let am = classify_message(
            v2_env("agent_message", r#"{ "text": "on it" }"#, "w2", "agent"),
            "2026-07-05T09:00:00Z",
        );
        assert_eq!(am.chat_kind, ChatKind::Session);
        assert_eq!(am.session_id, "w2"); // shared wrapper id, same as v1
        assert_eq!(am.source, "claude");
        assert_eq!(am.label.as_deref(), Some("my-repo"));
        assert_eq!(am.role, "agent");
        assert_eq!(am.body, "on it");
        assert_eq!(am.event_kind.as_deref(), Some("agent_message"));
        assert_eq!(am.timestamp, "2026-07-05T01:00:00Z"); // event ts wins
        assert!(am.advances_seq);

        // user_prompt defaults role to owner when the wire role is blank.
        let up = classify_message(
            v2_env(
                "user_prompt",
                r#"{ "text": "go", "source": "human" }"#,
                "w2",
                "",
            ),
            "2026-07-05T09:00:00Z",
        );
        assert_eq!(up.role, "owner");
        assert_eq!(up.body, "go");

        // tool_call → body is the display, tool_name + call_id captured.
        let tc = classify_message(
            v2_env(
                "tool_call",
                r#"{ "call_id": "c1", "tool_name": "bash", "tool_kind": "shell", "display": "ls" }"#,
                "w2",
                "agent",
            ),
            "2026-07-05T09:00:00Z",
        );
        assert_eq!(tc.event_kind.as_deref(), Some("tool_call"));
        assert_eq!(tc.body, "ls");
        assert_eq!(tc.tool_name.as_deref(), Some("bash"));
        assert_eq!(tc.call_id.as_deref(), Some("c1"));
        assert!(tc.advances_seq);

        // tool_result → body is the output, call_id correlates back to the call.
        let tr = classify_message(
            v2_env(
                "tool_result",
                r#"{ "call_id": "c1", "ok": true, "output": "file.rs", "output_bytes": 7 }"#,
                "w2",
                "agent",
            ),
            "2026-07-05T09:00:00Z",
        );
        assert_eq!(tr.event_kind.as_deref(), Some("tool_result"));
        assert_eq!(tr.body, "file.rs");
        assert_eq!(tr.call_id.as_deref(), Some("c1"));
        // A successful result carries ok=true / is_error=false, exit_code absent.
        assert_eq!(tr.ok, Some(true));
        assert_eq!(tr.is_error, Some(false));
        assert_eq!(tr.exit_code, None);

        // A FAILED tool_result carries the outcome so the renderer can flag it red.
        let tr_err = classify_message(
            v2_env(
                "tool_result",
                r#"{ "call_id": "c1", "ok": false, "is_error": true, "exit_code": 1, "output": "boom" }"#,
                "w2",
                "agent",
            ),
            "2026-07-05T09:00:00Z",
        );
        assert_eq!(tr_err.ok, Some(false));
        assert_eq!(tr_err.is_error, Some(true));
        assert_eq!(tr_err.exit_code, Some(1));

        // Older/partial payloads may omit `ok`; keep it unknown instead of
        // defaulting to failure.
        let tr_unknown = classify_message(
            v2_env(
                "tool_result",
                r#"{ "call_id": "c1", "is_error": false, "output": "done" }"#,
                "w2",
                "agent",
            ),
            "2026-07-05T09:00:00Z",
        );
        assert_eq!(tr_unknown.ok, None);
        assert_eq!(tr_unknown.is_error, Some(false));
        assert_eq!(tr_unknown.exit_code, None);

        // Non-tool_result rows leave the outcome fields unset.
        assert_eq!(tc.ok, None);
        assert_eq!(tc.is_error, None);
        assert_eq!(am.ok, None);
    }

    #[test]
    fn classifies_v2_status_and_approval_and_error_run_state() {
        // status → session-state-only: sets status_state/detail, does NOT advance seq.
        let st = classify_message(
            v2_env(
                "status",
                r#"{ "state": "running_tool", "detail": "compiling", "active_call_id": "c1" }"#,
                "w2",
                "agent",
            ),
            "2026-07-05T09:00:00Z",
        );
        assert_eq!(st.event_kind.as_deref(), Some("status"));
        assert_eq!(st.status_state.as_deref(), Some("running_tool"));
        assert_eq!(st.status_detail.as_deref(), Some("compiling"));
        assert_eq!(st.active_call_id.as_deref(), Some("c1"));
        assert!(!st.advances_seq, "status must not advance the wake ordinal");
        assert!(
            st.authoritative_status,
            "a status snapshot owns the run-state columns (may clear them)"
        );

        // approval_request → drives waiting_approval, keeps advancing (owner must act).
        let ar = classify_message(
            v2_env(
                "approval_request",
                r#"{ "call_id": "c9", "tool_name": "rm", "display": "rm -rf x" }"#,
                "w2",
                "agent",
            ),
            "2026-07-05T09:00:00Z",
        );
        assert_eq!(ar.status_state.as_deref(), Some("waiting_approval"));
        assert_eq!(ar.tool_name.as_deref(), Some("rm"));
        assert!(ar.advances_seq);
        assert!(
            !ar.authoritative_status,
            "approval sets status_state but is not a run-state snapshot (COALESCE)"
        );

        // error → errored run-state + body is the message.
        let er = classify_message(
            v2_env(
                "error",
                r#"{ "message": "boom", "fatal": true }"#,
                "w2",
                "agent",
            ),
            "2026-07-05T09:00:00Z",
        );
        assert_eq!(er.status_state.as_deref(), Some("errored"));
        assert_eq!(er.body, "boom");

        // lifecycle session_end → stopped, session-state-only.
        let lc = classify_message(
            v2_env("lifecycle", r#"{ "phase": "session_end" }"#, "w2", "agent"),
            "2026-07-05T09:00:00Z",
        );
        assert_eq!(lc.status_state.as_deref(), Some("stopped"));
        assert!(!lc.advances_seq);

        // unknown → dropped from the wake path, raw preserved as body. The raw
        // wire kind ("teleport") is normalized to the literal "unknown" so store
        // readers keep it out of the thread instead of leaking a forward kind.
        let uk = classify_message(
            v2_env("teleport", r#"{ "flux": 1 }"#, "w2", "agent"),
            "2026-07-05T09:00:00Z",
        );
        assert_eq!(uk.event_kind.as_deref(), Some("unknown"));
        assert!(!uk.advances_seq);
        assert!(uk.body.contains("flux"));
    }

    #[test]
    fn classifies_v2_session_info_as_session_enrichment() {
        // A full session_info: enrichment fields map onto the session record, the
        // title doubles as the row body, and it does NOT advance the wake ordinal
        // (same disposition as status/lifecycle).
        let si = classify_message(
            v2_env(
                "session_info",
                r#"{ "agent_address": "A1", "handle": "@alice", "title": "myrepo · feat/x",
                     "repo": "org/myrepo", "branch": "feat/x", "model": "claude-opus-4-8",
                     "capabilities": ["agent_message", "tool_call"], "resumed": false,
                     "started_at": "2026-07-08T00:00:00Z" }"#,
                "w2",
                "agent",
            ),
            "2026-07-05T09:00:00Z",
        );
        assert_eq!(si.chat_kind, ChatKind::Session);
        assert_eq!(si.session_id, "w2");
        assert_eq!(si.event_kind.as_deref(), Some("session_info"));
        assert!(
            !si.advances_seq,
            "session_info must not advance the wake ordinal"
        );
        assert!(!si.authoritative_status);
        assert_eq!(si.body, "myrepo · feat/x", "title doubles as the row body");
        assert_eq!(si.title.as_deref(), Some("myrepo · feat/x"));
        assert_eq!(si.model.as_deref(), Some("claude-opus-4-8"));
        assert_eq!(si.handle.as_deref(), Some("@alice"));
        assert_eq!(si.repo.as_deref(), Some("org/myrepo"));
        assert_eq!(si.branch.as_deref(), Some("feat/x"));
        assert_eq!(si.capabilities, vec!["agent_message", "tool_call"]);
    }

    #[test]
    fn session_info_model_falls_back_to_event_model() {
        // The payload omits `model`; the frame's `event.model` fills it (spec §2b:
        // "may also ride on event.model").
        let wire = format!(
            r#"{{
                "envelope_version": "tinyplace.harness.session.v2",
                "version": 2,
                "scope": {{ "type": "folder", "key": "my-repo", "cwd": "/w",
                           "wrapper_session_id": "w2", "harness_session_id": "h2" }},
                "harness": {{ "provider": "claude", "command": "claude", "argv": [] }},
                "event": {{ "id": "e1", "seq": 0, "ts": "2026-07-05T01:00:00Z",
                           "model": "opus-from-frame", "role": "agent",
                           "kind": "session_info",
                           "payload": {{ "agent_address": "A1" }} }},
                "source": {{ "path": "p", "record_type": "assistant" }}
            }}"#
        );
        let si = classify_message(wire, "2026-07-05T09:00:00Z");
        assert_eq!(si.model.as_deref(), Some("opus-from-frame"));
        assert!(si.title.is_none());
        assert!(si.capabilities.is_empty());
    }

    #[test]
    fn persisting_session_info_enriches_session_lazily_and_idempotently() {
        let tmp = tempfile::tempdir().unwrap();
        // session_info is the FIRST event for this session — it must lazy-create
        // the record (enrichment, not a prerequisite), populate the metadata, and
        // stamp last_seq = 0 (no spurious wake).
        let si = classify_message(
            v2_env(
                "session_info",
                r#"{ "agent_address": "A1", "handle": "@alice", "title": "Intro",
                     "repo": "org/myrepo", "branch": "feat/x", "model": "opus",
                     "capabilities": ["agent_message"], "resumed": false,
                     "started_at": "2026-07-08T00:00:00Z" }"#,
                "w2",
                "agent",
            ),
            "2026-07-05T09:00:00Z",
        );
        assert!(persist_message(tmp.path(), "si1", "@peer", &si, "now")
            .unwrap()
            .is_some());
        // Re-persisting the SAME event id is idempotent (dedup on the relay id).
        assert!(persist_message(tmp.path(), "si1", "@peer", &si, "now")
            .unwrap()
            .is_none());

        store::with_connection(tmp.path(), |c| {
            let s = store::load_session(c, "@peer", "w2")?.expect("session lazy-created");
            assert_eq!(s.title.as_deref(), Some("Intro"));
            assert_eq!(s.model.as_deref(), Some("opus"));
            assert_eq!(s.handle.as_deref(), Some("@alice"));
            assert_eq!(s.repo.as_deref(), Some("org/myrepo"));
            assert_eq!(s.branch.as_deref(), Some("feat/x"));
            assert_eq!(s.capabilities, vec!["agent_message".to_string()]);
            assert_eq!(s.last_seq, 0, "session_info must not advance last_seq");
            assert_eq!(store::count_messages(c, "@peer", "w2")?, 1);
            Ok(())
        })
        .unwrap();

        // A reconnect re-intro (resumed=true, NEW event id, refreshed title)
        // UPDATES the record rather than duplicating it, and a content event in
        // between never wipes the intro metadata (COALESCE).
        let content = classify_message(
            v2_env("agent_message", r#"{ "text": "working" }"#, "w2", "agent"),
            "2026-07-05T09:01:00Z",
        );
        assert!(persist_message(tmp.path(), "m1", "@peer", &content, "now")
            .unwrap()
            .is_some());
        let resumed = classify_message(
            v2_env(
                "session_info",
                r#"{ "agent_address": "A1", "title": "Intro v2", "resumed": true,
                     "started_at": "2026-07-08T01:00:00Z" }"#,
                "w2",
                "agent",
            ),
            "2026-07-05T09:02:00Z",
        );
        assert!(persist_message(tmp.path(), "si2", "@peer", &resumed, "now")
            .unwrap()
            .is_some());

        store::with_connection(tmp.path(), |c| {
            let s = store::load_session(c, "@peer", "w2")?.expect("session");
            assert_eq!(
                s.title.as_deref(),
                Some("Intro v2"),
                "resumed intro refreshes title"
            );
            // Untouched intro fields survive the content event + the thin re-intro.
            assert_eq!(s.model.as_deref(), Some("opus"));
            assert_eq!(s.capabilities, vec!["agent_message".to_string()]);
            // Only the content event advanced the ordinal (seq 1); both session_info
            // rows persist at seq 0.
            assert_eq!(s.last_seq, 1);
            assert_eq!(store::count_messages(c, "@peer", "w2")?, 3);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn v1_and_v2_envelopes_coexist_in_one_session_model() {
        // During rollout a wrapped harness may emit v1 then v2 under the SAME
        // shared wrapper id. Both classify to ChatKind::Session under the same
        // session_id and persist into one session — proving the coexistence seam.
        let tmp = tempfile::tempdir().unwrap();
        let v1 = classify_message(ENVELOPE.to_string(), "2026-07-02T09:00:00Z"); // wrapper w1
        let v2 = classify_message(
            v2_env("agent_message", r#"{ "text": "v2 line" }"#, "w1", "agent"),
            "2026-07-05T09:00:00Z",
        );
        assert_eq!(v1.session_id, "w1");
        assert_eq!(v2.session_id, "w1");

        assert!(persist_message(tmp.path(), "m-v1", "@peer", &v1, "now")
            .unwrap()
            .is_some());
        assert!(persist_message(tmp.path(), "m-v2", "@peer", &v2, "now")
            .unwrap()
            .is_some());
        store::with_connection(tmp.path(), |c| {
            // Both rows land in the single "w1" session, seqs monotonic 1,2.
            assert_eq!(store::count_messages(c, "@peer", "w1")?, 2);
            let seqs: Vec<i64> = store::list_recent_messages(c, "@peer", "w1", 10)?
                .iter()
                .map(|m| m.seq)
                .collect();
            assert_eq!(seqs, vec![1, 2]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn persisting_v2_status_updates_session_row_without_advancing_seq() {
        let tmp = tempfile::tempdir().unwrap();
        // A content message establishes the session at seq 1.
        let msg = classify_message(
            v2_env("agent_message", r#"{ "text": "working" }"#, "w2", "agent"),
            "2026-07-05T09:00:00Z",
        );
        assert!(persist_message(tmp.path(), "m1", "@peer", &msg, "now")
            .unwrap()
            .is_some());
        // A status event follows: it lands a (deduped) row + updates the session
        // status columns, but last_seq must stay at 1 (no spurious wake).
        let status = classify_message(
            v2_env(
                "status",
                r#"{ "state": "running", "detail": "still working", "active_call_id": "c3" }"#,
                "w2",
                "agent",
            ),
            "2026-07-05T09:00:00Z",
        );
        assert!(persist_message(tmp.path(), "m2", "@peer", &status, "now")
            .unwrap()
            .is_some());

        store::with_connection(tmp.path(), |c| {
            let session = store::load_session(c, "@peer", "w2")?.expect("session exists");
            assert_eq!(session.status_state.as_deref(), Some("running"));
            assert_eq!(session.current_detail.as_deref(), Some("still working"));
            assert_eq!(session.active_call_id.as_deref(), Some("c3"));
            assert_eq!(session.last_seq, 1, "status must not advance last_seq");
            // The status row is persisted (id-dedupe) with its event_kind + seq 0.
            let msgs = store::list_recent_messages(c, "@peer", "w2", 10)?;
            let status_row = msgs
                .iter()
                .find(|m| m.event_kind.as_deref() == Some("status"))
                .expect("status row persisted");
            assert_eq!(status_row.seq, 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_later_status_snapshot_clears_stale_detail_and_active_call_id() {
        let tmp = tempfile::tempdir().unwrap();
        // running_tool: detail + active_call_id populated.
        let running = classify_message(
            v2_env(
                "status",
                r#"{ "state": "running_tool", "detail": "compiling", "active_call_id": "c1" }"#,
                "w2",
                "agent",
            ),
            "2026-07-05T09:00:00Z",
        );
        assert!(persist_message(tmp.path(), "s1", "@peer", &running, "now")
            .unwrap()
            .is_some());
        store::with_connection(tmp.path(), |c| {
            let s = store::load_session(c, "@peer", "w2")?.expect("session");
            assert_eq!(s.current_detail.as_deref(), Some("compiling"));
            assert_eq!(s.active_call_id.as_deref(), Some("c1"));
            Ok(())
        })
        .unwrap();

        // idle: the harness went quiet — detail + active_call_id are absent. The
        // authoritative snapshot must CLEAR them, not keep the stale "compiling"/c1
        // (the COALESCE upsert alone would preserve them — the bug this fixes).
        let idle = classify_message(
            v2_env("status", r#"{ "state": "idle" }"#, "w2", "agent"),
            "2026-07-05T09:01:00Z",
        );
        assert!(persist_message(tmp.path(), "s2", "@peer", &idle, "now")
            .unwrap()
            .is_some());
        store::with_connection(tmp.path(), |c| {
            let s = store::load_session(c, "@peer", "w2")?.expect("session");
            assert_eq!(s.status_state.as_deref(), Some("idle"));
            assert_eq!(s.current_detail, None, "stale detail must be cleared");
            assert_eq!(
                s.active_call_id, None,
                "stale active_call_id must be cleared"
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_content_event_never_wipes_a_live_status() {
        let tmp = tempfile::tempdir().unwrap();
        // Establish a live run-state via a status snapshot.
        let running = classify_message(
            v2_env(
                "status",
                r#"{ "state": "running_tool", "detail": "compiling", "active_call_id": "c1" }"#,
                "w2",
                "agent",
            ),
            "2026-07-05T09:00:00Z",
        );
        assert!(persist_message(tmp.path(), "s1", "@peer", &running, "now")
            .unwrap()
            .is_some());
        // A content event (agent_message) carries no run-state — it must COALESCE,
        // preserving the live status rather than nulling it.
        let msg = classify_message(
            v2_env(
                "agent_message",
                r#"{ "text": "still going" }"#,
                "w2",
                "agent",
            ),
            "2026-07-05T09:00:30Z",
        );
        assert!(persist_message(tmp.path(), "m1", "@peer", &msg, "now")
            .unwrap()
            .is_some());
        store::with_connection(tmp.path(), |c| {
            let s = store::load_session(c, "@peer", "w2")?.expect("session");
            assert_eq!(s.status_state.as_deref(), Some("running_tool"));
            assert_eq!(s.current_detail.as_deref(), Some("compiling"));
            assert_eq!(s.active_call_id.as_deref(), Some("c1"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn persisting_v2_error_records_last_error() {
        let tmp = tempfile::tempdir().unwrap();
        let err = classify_message(
            v2_env(
                "error",
                r#"{ "message": "rate limited", "fatal": false }"#,
                "w2",
                "agent",
            ),
            "2026-07-05T09:00:00Z",
        );
        assert!(persist_message(tmp.path(), "e1", "@peer", &err, "now")
            .unwrap()
            .is_some());
        store::with_connection(tmp.path(), |c| {
            assert_eq!(
                store::kv_get(c, "orchestration:last_error")?.as_deref(),
                Some("rate limited")
            );
            let session = store::load_session(c, "@peer", "w2")?.unwrap();
            assert_eq!(session.status_state.as_deref(), Some("errored"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn persist_message_is_idempotent_and_buckets_by_session() {
        let tmp = tempfile::tempdir().unwrap();
        let session = classify_message(ENVELOPE.to_string(), "2026-07-02T09:00:00Z");
        let master = classify_message("hi".to_string(), "2026-07-02T09:00:00Z");

        assert!(persist_message(tmp.path(), "m1", "@peer", &session, "now")
            .unwrap()
            .is_some());
        // Replay of the same relay id does not double-insert.
        assert!(persist_message(tmp.path(), "m1", "@peer", &session, "now")
            .unwrap()
            .is_none());
        assert!(persist_message(tmp.path(), "m2", "@peer", &master, "now")
            .unwrap()
            .is_some());

        store::with_connection(tmp.path(), |c| {
            assert_eq!(store::count_messages(c, "@peer", "w1")?, 1); // per-pair wrapper id
            assert_eq!(store::count_messages(c, "@peer", "master")?, 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn persist_stamps_monotonic_ingest_seq_so_line_zero_dms_still_wake() {
        // Regression for the silent drop (#4583). A wrapped Claude harness stamps
        // `line = 0` on EVERY DM, so pre-fix both messages classified to seq 0;
        // under the shared wrapper-session key the wake cursor pinned at 0 and the
        // second message was persisted + acked but never woke the graph (no reply).
        // Persist must ignore the wire `line` and stamp a strictly-increasing
        // per-(agent, session) ingest ordinal so `last_seq` advances past the cursor.
        let tmp = tempfile::tempdir().unwrap();
        let line_zero = || {
            classify_message(
                ENVELOPE.replace("\"line\": 7", "\"line\": 0"),
                "2026-07-02T09:00:00Z",
            )
        };
        let first = line_zero();
        let second = line_zero();
        assert_eq!(first.seq, 0); // wire line is 0 for both …
        assert_eq!(second.seq, 0);

        assert!(persist_message(tmp.path(), "mA", "@peer", &first, "now")
            .unwrap()
            .is_some());
        assert!(persist_message(tmp.path(), "mB", "@peer", &second, "now")
            .unwrap()
            .is_some());

        store::with_connection(tmp.path(), |c| {
            // … but the persisted seqs are monotonic ingest ordinals 1 and 2, and
            // last_seq advanced to 2 — so a wake cursor left at 1 sees new work.
            assert_eq!(store::count_messages(c, "@peer", "w1")?, 2);
            assert_eq!(store::session_last_seq(c, "@peer", "w1")?, Some(2));
            let seqs: Vec<i64> = store::list_recent_messages(c, "@peer", "w1", 10)?
                .iter()
                .map(|m| m.seq)
                .collect();
            assert_eq!(seqs, vec![1, 2]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn unrecoverable_decrypt_errors_are_dead_lettered_but_transient_ones_are_not() {
        // Signal-layer failures (prefixed "decryption failed:" by decrypt_envelope)
        // are permanent for the envelope and must be dropped so they can't poison
        // the drain loop forever — this is the "No session" case from the bug.
        assert!(is_unrecoverable_decrypt_error(
            "decryption failed: invalid argument: No session for De6RHrMj6eDqX1WBTXk11sks"
        ));
        assert!(is_unrecoverable_decrypt_error("decryption failed: bad MAC"));
        // Transient failures must be retried, not swallowed.
        assert!(!is_unrecoverable_decrypt_error(
            "HTTP 503: /keys/abc/bundle"
        ));
        assert!(!is_unrecoverable_decrypt_error(
            "identity key: store unavailable"
        ));
        assert!(!is_unrecoverable_decrypt_error("messages.list: timeout"));
    }

    #[test]
    fn prekey_bundles_are_ordered_before_ciphertext_preserving_relative_order() {
        let env = |id: &str, ty: &str| -> tinyplace::types::MessageEnvelope {
            serde_json::from_value(serde_json::json!({ "id": id, "type": ty })).unwrap()
        };
        // Delivered order interleaves a CIPHERTEXT before the PREKEY_BUNDLE that
        // establishes its session — the ordering race the fix removes.
        let mut batch = vec![
            env("c1", "CIPHERTEXT"),
            env("pk", "PREKEY_BUNDLE"),
            env("c2", "CIPHERTEXT"),
        ];
        order_prekey_bundles_first(&mut batch);
        let ids: Vec<&str> = batch.iter().map(|m| m.id.as_str()).collect();
        // PREKEY_BUNDLE first; CIPHERTEXT keep their delivered order (stable sort).
        assert_eq!(ids, vec!["pk", "c1", "c2"]);
    }
}

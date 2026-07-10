//! Client-side executor for device-bound orchestration effects pushed by the
//! hosted brain over the socket.
//!
//! The backend computes a wake cycle and, when it wants the device to act,
//! delivers an effect frame (`orch:effect:send_dm`, `orch:effect:evict`, …). The
//! device runs the effect against its local Signal keys / memory and acks with
//! `orch:effect:result { callId, ok, error? }`. Delivery is at-least-once and the
//! client dedupes on `callId` (a `send_dm` whose ack was already latched
//! server-side is a no-op here — see [`is_duplicate_call`]).

use std::collections::HashSet;
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::{json, Value};

const LOG: &str = "orchestration";

/// A `send_dm` effect: relay `body` to `counterpartAgentId` over Signal, wrapping
/// it in `sessionId`'s session envelope so the peer threads the reply.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendDmEffect {
    #[serde(default)]
    pub cycle_id: String,
    pub call_id: String,
    pub counterpart_agent_id: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub body: String,
}

/// Parse an `orch:effect:send_dm` frame. Pure.
pub fn parse_send_dm(data: &Value) -> Result<SendDmEffect, String> {
    serde_json::from_value(data.clone()).map_err(|e| format!("parse send_dm: {e}"))
}

/// Build the `orch:effect:result` ack frame the device sends back. Pure.
pub fn effect_result_frame(call_id: &str, ok: bool, error: Option<&str>) -> Value {
    json!({ "callId": call_id, "ok": ok, "error": error })
}

/// The device-tool manifest declared to the hosted brain on socket connect
/// (`orch:register_tools`). These are **queryable** tools the reasoning loop may
/// call mid-cycle (results feed back), distinct from the terminal `send_dm`
/// effect. Phase 2 seeds it with a read-only status probe; local-workspace tools
/// grow this list as they are wired to the device tool dispatcher.
pub fn device_tool_manifest() -> Value {
    json!({
        "tools": [
            {
                "name": "device_status",
                "description": "Report this device's app version and platform.",
                "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
            },
            {
                "name": "run_local_agent",
                "description": "Spawn a local device sub-agent (e.g. code_executor for repo/shell/file work, researcher, tools_agent) on the user's own machine for background work, and return an acknowledgement. LOCAL-EXECUTION: only runs for a Master-chat cycle (the human ↔ their own OpenHuman); it is refused for any agent-to-agent cycle.",
                "inputSchema": {
                    "type": "object",
                    "required": ["agent_id", "prompt"],
                    "properties": {
                        "agent_id": { "type": "string", "description": "Local sub-agent id, e.g. code_executor, researcher, tools_agent." },
                        "prompt": { "type": "string", "description": "Clear, self-contained instruction for the sub-agent." },
                        "context": { "type": "string", "description": "Optional context blob from prior results." }
                    },
                    "additionalProperties": false
                }
            }
        ]
    })
}

/// A device tool call pushed by the hosted brain (`orch:tool_call`). Run it
/// locally and return the result over `orch:tool_result`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallFrame {
    #[serde(default)]
    pub cycle_id: String,
    pub call_id: String,
    pub name: String,
    #[serde(default)]
    pub args: Value,
}

/// Parse an `orch:tool_call` frame. Pure.
pub fn parse_tool_call(data: &Value) -> Result<ToolCallFrame, String> {
    serde_json::from_value(data.clone()).map_err(|e| format!("parse tool_call: {e}"))
}

/// Build the `orch:tool_result` frame returned to the hosted brain. Pure.
pub fn tool_result_frame(call_id: &str, ok: bool, result: Value, error: Option<&str>) -> Value {
    json!({ "callId": call_id, "ok": ok, "result": result, "error": error })
}

/// Run a device-declared tool locally and return its result. `device_status` is
/// read-only; `run_local_agent` executes local workspace work and is therefore
/// **gated**: it runs only when `cycle_id` belongs to a Master-chat cycle
/// (device-authoritative origin, see [`super::exec_gate`]). The gate is enforced
/// here — on the device that holds the capability — so a prompt-injected or
/// compromised cloud brain cannot induce local execution for an A2A cycle.
pub async fn dispatch_device_tool(
    name: &str,
    args: &Value,
    cycle_id: &str,
) -> Result<Value, String> {
    if super::exec_gate::is_local_execution_tool(name)
        && !super::exec_gate::cycle_is_master(cycle_id)
    {
        log::warn!(
            target: LOG,
            "[orchestration] device_tool.denied name={name} cycle={cycle_id} reason=non_master_origin"
        );
        return Err(format!(
            "device tool '{name}' denied: local execution is restricted to the Master chat"
        ));
    }
    match name {
        "device_status" => Ok(json!({
            "version": env!("CARGO_PKG_VERSION"),
            "platform": std::env::consts::OS,
        })),
        "run_local_agent" => run_local_agent(args, cycle_id).await,
        other => Err(format!("unknown device tool: {other}")),
    }
}

/// The gated `run_local_agent` device tool. Reached only after the Master-chat
/// gate in [`dispatch_device_tool`] has passed.
///
/// **Async model:** we do NOT block the wake cycle on a (potentially long) local
/// sub-agent. We fire it in the background and return an immediate `accepted`
/// ack — which the hosted brain sees as `orch:tool_result` well inside the
/// device-tool timeout. When the sub-agent finishes, [`run_local_agent_and_forward`]
/// pushes its result up as a fresh `tool_completion` event, which wakes a NEW
/// cycle that reasons over the result. So the original cycle is never blocked,
/// and the result still lands back in the brain via the follow-up cycle.
async fn run_local_agent(args: &Value, cycle_id: &str) -> Result<Value, String> {
    let (counterpart, session_id) = super::exec_gate::cycle_target(cycle_id)
        .ok_or_else(|| "run_local_agent: unknown cycle origin".to_string())?;
    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let prompt = args
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if agent_id.is_empty() || prompt.is_empty() {
        return Err("run_local_agent: `agent_id` and `prompt` are required".to_string());
    }
    let task_id = cycle_id.to_string();
    let run_args = args.clone();
    let bg_task_id = task_id.clone();
    tokio::spawn(async move {
        if let Err(e) =
            run_local_agent_and_forward(&counterpart, &session_id, &bg_task_id, &agent_id, run_args)
                .await
        {
            log::warn!(
                target: LOG,
                "[orchestration] run_local_agent.forward_failed task={bg_task_id}: {e}"
            );
        }
    });
    Ok(json!({
        "accepted": true,
        "taskId": task_id,
        "status": "running",
        "note": "local sub-agent started; its result will arrive as a follow-up tool_completion event."
    }))
}

/// Background half of `run_local_agent`: run the local sub-agent to completion,
/// then forward its result up as a `tool_completion` event on the originating
/// session (which the backend wakes a fresh cycle for).
async fn run_local_agent_and_forward(
    counterpart: &str,
    session_id: &str,
    task_id: &str,
    agent_id: &str,
    run_args: Value,
) -> Result<(), String> {
    use crate::openhuman::tools::traits::Tool;
    // 1. Run the local sub-agent synchronously to completion (real output).
    let tool = crate::openhuman::agent_orchestration::tools::SpawnSubagentTool::new();
    // Convert an invocation error into a failure completion rather than bailing:
    // the hosted brain must always learn the outcome (success OR failure) via the
    // forwarded `tool_completion`, never be left with no follow-up at all.
    let (ok, output) = match tool.execute(run_args).await {
        Ok(result) => (!result.is_error, result.output()),
        Err(e) => (false, format!("sub-agent invocation error: {e}")),
    };
    let body = format!(
        "[local sub-agent `{agent_id}` task {task_id} {}]\n{output}",
        if ok { "completed" } else { "failed" }
    );

    // 2. Persist the completion into the render cache (allocates a monotonic seq)
    //    and forward it as a `tool_completion` event → backend wakes a new cycle.
    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| format!("config load: {e}"))?;
    let now = chrono::Utc::now().to_rfc3339();
    let seq = super::store::with_connection(&config.workspace_dir, |conn| {
        let seq = super::store::next_session_seq(conn, counterpart, session_id)?;
        super::store::insert_message(
            conn,
            &super::types::OrchestrationMessage {
                id: format!("tool-completion:{task_id}:{seq}"),
                agent_id: counterpart.to_string(),
                session_id: session_id.to_string(),
                chat_kind: super::types::ChatKind::Master,
                role: "system".to_string(),
                body: body.clone(),
                timestamp: now.clone(),
                seq,
                ..Default::default()
            },
        )?;
        Ok(seq)
    })
    .map_err(|e| format!("persist completion: {e}"))?;

    let ts = super::wire::parse_ts_ms(&now).unwrap_or(0);
    let envelope = super::wire::OrchestrationEventEnvelopeWire::build(
        counterpart,
        session_id,
        seq,
        "system",
        counterpart,
        &body,
        ts,
        "tool_completion",
    );
    super::cloud::push_event(&config, &envelope).await?;
    log::debug!(
        target: LOG,
        "[orchestration] run_local_agent.forwarded task={task_id} session={session_id} seq={seq} ok={ok}"
    );
    Ok(())
}

/// Handle an inbound `orch:tool_call` frame end-to-end: parse → dispatch →
/// build the result frame. Returns `(callId, resultFrame)` to emit, or `None`
/// when the frame is unparseable. Async because a local sub-agent spawn is.
pub async fn handle_tool_call(data: &Value) -> Option<(String, Value)> {
    let frame = match parse_tool_call(data) {
        Ok(f) => f,
        Err(e) => {
            log::warn!(target: LOG, "[orchestration] tool_call.parse_failed: {e}");
            return None;
        }
    };
    let (ok, result, error) =
        match dispatch_device_tool(&frame.name, &frame.args, &frame.cycle_id).await {
            Ok(value) => (true, value, None),
            Err(e) => (false, Value::Null, Some(e)),
        };
    Some((
        frame.call_id.clone(),
        tool_result_frame(&frame.call_id, ok, result, error.as_deref()),
    ))
}

// ── callId dedupe (at-least-once delivery guard) ──────────────────────────────

static SEEN_CALL_IDS: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Bound on retained call ids. At the cap the window is cleared wholesale (a coarse
/// TTL) so the guard can't grow without limit now that both send_dm and evict feed
/// it; a redelivery for an effect older than this many claims may then re-execute,
/// which is safe (effects are idempotent) and vanishingly rare.
const SEEN_CALL_IDS_CAP: usize = 16_384;

/// Claim `call_id` for execution and report whether it was already claimed (i.e. a
/// redelivered effect). Claiming on entry — rather than on success — is deliberate:
/// it stops a redelivery that arrives *while the first attempt is still running* from
/// executing the effect a second time. A claim whose effect ultimately FAILS is
/// released via [`release_call`] so the hosted brain's redelivery re-runs it; a claim
/// whose effect succeeds stays latched so the redelivery is re-acked idempotently
/// without re-executing.
pub fn is_duplicate_call(call_id: &str) -> bool {
    let mut guard = SEEN_CALL_IDS.lock().unwrap_or_else(|p| p.into_inner());
    let set = guard.get_or_insert_with(HashSet::new);
    if set.len() >= SEEN_CALL_IDS_CAP {
        set.clear();
    }
    !set.insert(call_id.to_string())
}

/// Release a `call_id` claimed by [`is_duplicate_call`] after its effect FAILED, so a
/// subsequent redelivery re-executes it instead of the guard re-acking a stale
/// `ok:true` and silently dropping the lost work.
pub fn release_call(call_id: &str) {
    let mut guard = SEEN_CALL_IDS.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(set) = guard.as_mut() {
        set.remove(call_id);
    }
}

/// Wrap the reply body for `session_id`. Empty/`master`/`subconscious` sessions
/// send the plain body; a real harness session is wrapped in a v1 envelope so
/// the peer threads it. Delegates to the shared orchestration helper.
fn outgoing_plaintext(session_id: &str, body: &str) -> Result<String, String> {
    if session_id.is_empty() {
        return Ok(body.to_string());
    }
    super::ops::session_send_plaintext(session_id, body).map_err(|e| e.to_string())
}

/// Execute a `send_dm` effect: wrap + send over Signal via the existing
/// tinyplace transport. The device's Signal keys never leave the machine.
pub async fn execute_send_dm(effect: &SendDmEffect) -> Result<(), String> {
    let plaintext = outgoing_plaintext(&effect.session_id, &effect.body)?;
    let mut params = serde_json::Map::new();
    params.insert(
        "recipient".to_string(),
        Value::from(effect.counterpart_agent_id.clone()),
    );
    params.insert("plaintext".to_string(), Value::from(plaintext));
    crate::openhuman::tinyplace::handle_tinyplace_signal_send_message(params)
        .await
        .map_err(|e| format!("signal send: {e}"))?;
    Ok(())
}

/// A self-directed reply (the human↔OpenHuman Master chat, or the subconscious
/// window) — rendered into the local cache, never Signal-sent to a peer. The
/// hosted brain ships every terminal reply as a `send_dm`; the device decides
/// whether that reply is for a peer or for the user's own window.
fn is_self_session(session_id: &str) -> bool {
    session_id.is_empty() || session_id == "master" || session_id == "subconscious"
}

/// Persist a hosted-authored reply into the local render cache as an `assistant`
/// message and nudge the renderer, so the reply shows in its window even though
/// the reasoning ran server-side. Idempotent by `call_id` (`INSERT OR IGNORE`),
/// so a redelivered effect never doubles a row.
async fn persist_reply(
    agent_id: &str,
    session_id: &str,
    chat_kind: super::types::ChatKind,
    call_id: &str,
    body: &str,
) -> Result<(), String> {
    use super::types::{OrchestrationMessage, OrchestrationSession};
    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| format!("config load: {e}"))?;
    let now = chrono::Utc::now().to_rfc3339();
    let message_id = format!("reply:{call_id}");
    let agent_owned = agent_id.to_string();
    let session_owned = session_id.to_string();
    let body_owned = body.to_string();
    let now_owned = now.clone();
    super::store::with_connection(&config.workspace_dir, move |conn| {
        // Allocate the per-session seq and persist the reply in one immediate txn so
        // two concurrent self-replies on the same (agent_id, session_id) can't read
        // the same `MAX(seq)+1` and persist a duplicate ordinal (matches `ingest_one`).
        super::store::in_immediate_txn(conn, |conn| {
            let seq = super::store::next_session_seq(conn, &agent_owned, &session_owned)?;
            super::store::upsert_session(
                conn,
                &OrchestrationSession {
                    session_id: session_owned.clone(),
                    agent_id: agent_owned.clone(),
                    source: chat_kind.as_str().to_string(),
                    last_seq: seq,
                    created_at: now_owned.clone(),
                    last_message_at: now_owned.clone(),
                    ..Default::default()
                },
            )?;
            super::store::insert_message(
                conn,
                &OrchestrationMessage {
                    id: message_id.clone(),
                    agent_id: agent_owned.clone(),
                    session_id: session_owned.clone(),
                    chat_kind,
                    role: "assistant".to_string(),
                    body: body_owned.clone(),
                    timestamp: now_owned.clone(),
                    seq,
                    ..Default::default()
                },
            )?;
            Ok(())
        })
    })
    .map_err(|e| format!("persist reply: {e}"))?;
    super::bus::notify_orchestration_message(agent_id, session_id, chat_kind.as_str());
    Ok(())
}

/// Handle an inbound `orch:effect:send_dm` frame end-to-end.
///
/// The hosted brain ships every terminal reply here. The device:
/// - mirrors the reply body into the local render cache (so the window shows it
///   regardless of the peer-send outcome), then
/// - for a **self** session (Master / subconscious) that is the whole job — the
///   reply is rendered, never Signal-sent;
/// - for a **peer** session it is also sent over Signal to the counterpart.
///
/// Returns `(callId, ackFrame)` for the caller to emit over `orch:effect:result`,
/// or `None` when the frame is unparseable (nothing to ack).
pub async fn handle_send_dm(data: &Value) -> Option<(String, Value)> {
    let effect = match parse_send_dm(data) {
        Ok(e) => e,
        Err(e) => {
            log::warn!(target: LOG, "[orchestration] effect.send_dm.parse_failed: {e}");
            return None;
        }
    };

    if is_duplicate_call(&effect.call_id) {
        log::debug!(
            target: LOG,
            "[orchestration] effect.send_dm.duplicate call_id={} (re-acking)",
            effect.call_id
        );
        return Some((
            effect.call_id.clone(),
            effect_result_frame(&effect.call_id, true, None),
        ));
    }

    let self_session = is_self_session(&effect.session_id);
    // Where the reply is rendered locally. Self replies land in the user's own
    // Master/subconscious window; peer replies land in that peer's session.
    let (cache_agent, cache_session, chat_kind) = if self_session {
        let session = if effect.session_id.is_empty() {
            "master"
        } else {
            &effect.session_id
        };
        (
            super::types::LOCAL_MASTER_AGENT.to_string(),
            session.to_string(),
            super::types::ChatKind::from_str(session),
        )
    } else {
        (
            effect.counterpart_agent_id.clone(),
            effect.session_id.clone(),
            super::types::ChatKind::Session,
        )
    };

    let persist_res = persist_reply(
        &cache_agent,
        &cache_session,
        chat_kind,
        &effect.call_id,
        &effect.body,
    )
    .await;

    // A self reply is terminal here — no peer to Signal. Ack reflects the local
    // render outcome so a rare cache-write failure is visible server-side.
    if self_session {
        return Some(match persist_res {
            Ok(()) => {
                log::debug!(
                    target: LOG,
                    "[orchestration] effect.send_dm.self_reply call_id={} session={}",
                    effect.call_id,
                    cache_session
                );
                (
                    effect.call_id.clone(),
                    effect_result_frame(&effect.call_id, true, None),
                )
            }
            Err(e) => {
                log::warn!(target: LOG, "[orchestration] effect.send_dm.self_persist_failed call_id={}: {e}", effect.call_id);
                // Un-claim so a redelivery retries the local render.
                release_call(&effect.call_id);
                (
                    effect.call_id.clone(),
                    effect_result_frame(&effect.call_id, false, Some(&e)),
                )
            }
        });
    }

    // Peer reply: the cache mirror is best-effort (log on failure), but the ack
    // reflects the Signal send — that is what the hosted outbox is tracking.
    if let Err(e) = persist_res {
        log::warn!(target: LOG, "[orchestration] effect.send_dm.cache_mirror_failed call_id={}: {e}", effect.call_id);
    }
    let (ok, error) = match execute_send_dm(&effect).await {
        Ok(()) => {
            log::debug!(
                target: LOG,
                "[orchestration] effect.send_dm.sent call_id={} session={}",
                effect.call_id,
                effect.session_id
            );
            (true, None)
        }
        Err(e) => {
            log::warn!(target: LOG, "[orchestration] effect.send_dm.failed call_id={}: {e}", effect.call_id);
            (false, Some(e))
        }
    };
    if !ok {
        // Un-claim so a redelivery re-sends instead of re-acking a stale success.
        release_call(&effect.call_id);
    }
    Some((
        effect.call_id.clone(),
        effect_result_frame(&effect.call_id, ok, error.as_deref()),
    ))
}

// ── evict effect (context-guard → local memory RAG) ───────────────────────────

/// One evicted compressed-history entry the hosted context-guard is asking the
/// device to mirror into its local memory RAG so the summary stays retrievable
/// offline after the server drops it from the wake context window.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvictEntry {
    #[serde(default)]
    pub cycle_id: String,
    #[serde(default)]
    pub summary: String,
}

/// An `orch:effect:evict` effect. Backend frame:
/// `{ cycleId, callId, sessionId, entries: [{ cycleId, summary }] }`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvictEffect {
    #[serde(default)]
    pub cycle_id: String,
    pub call_id: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub entries: Vec<EvictEntry>,
}

/// Parse an `orch:effect:evict` frame. Pure.
pub fn parse_evict(data: &Value) -> Result<EvictEffect, String> {
    serde_json::from_value(data.clone()).map_err(|e| format!("parse evict: {e}"))
}

/// Stable, idempotent RAG source id for an evicted entry. The ingest pipeline
/// gates on this key, so a redelivered evict (or a replayed cycle) re-ingests
/// nothing even beyond the `callId` dedupe.
fn evict_source_id(session_id: &str, cycle_id: &str) -> String {
    let session = if session_id.is_empty() {
        "master"
    } else {
        session_id
    };
    format!("orch_evict:{session}:{cycle_id}")
}

/// Fold each evicted summary into local memory RAG via the standard ingest
/// pipeline. The device's memory never leaves the machine — only the hosted
/// brain's own compressed summary text (which it just sent us) is stored.
pub async fn execute_evict(effect: &EvictEffect) -> Result<(), String> {
    use crate::openhuman::memory::ingest_pipeline::ingest_document_with_scope;
    use crate::openhuman::memory_sync::canonicalize::document::DocumentInput;

    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| format!("config load: {e}"))?;

    let mut ingested = 0usize;
    for entry in &effect.entries {
        if entry.summary.trim().is_empty() {
            continue;
        }
        let source_id = evict_source_id(&effect.session_id, &entry.cycle_id);
        let doc = DocumentInput {
            provider: "orchestration".to_string(),
            title: format!("Evicted orchestration summary {}", entry.cycle_id),
            body: entry.summary.clone(),
            modified_at: chrono::Utc::now(),
            source_ref: Some(source_id.clone()),
        };
        ingest_document_with_scope(
            &config,
            &source_id,
            "user",
            vec!["orchestration".to_string(), "evicted".to_string()],
            doc,
            Some("orchestration/evicted".to_string()),
        )
        .await
        .map_err(|e| format!("evict ingest cycle={}: {e}", entry.cycle_id))?;
        ingested += 1;
    }
    log::debug!(
        target: LOG,
        "[orchestration] effect.evict.ingested count={ingested} session={}",
        effect.session_id
    );
    Ok(())
}

/// Handle an inbound `orch:effect:evict` frame end-to-end: parse → dedupe →
/// mirror into RAG → produce the ack frame. Returns `(callId, ackFrame)` for the
/// caller to emit over `orch:effect:result`, or `None` when unparseable.
pub async fn handle_evict(data: &Value) -> Option<(String, Value)> {
    let effect = match parse_evict(data) {
        Ok(e) => e,
        Err(e) => {
            log::warn!(target: LOG, "[orchestration] effect.evict.parse_failed: {e}");
            return None;
        }
    };

    if is_duplicate_call(&effect.call_id) {
        log::debug!(
            target: LOG,
            "[orchestration] effect.evict.duplicate call_id={} (re-acking)",
            effect.call_id
        );
        return Some((
            effect.call_id.clone(),
            effect_result_frame(&effect.call_id, true, None),
        ));
    }

    let (ok, error) = match execute_evict(&effect).await {
        Ok(()) => (true, None),
        Err(e) => {
            log::warn!(target: LOG, "[orchestration] effect.evict.failed call_id={}: {e}", effect.call_id);
            (false, Some(e))
        }
    };
    if !ok {
        // Un-claim so a redelivery re-runs the eviction instead of re-acking a stale
        // success and losing the summary the brain asked to evict.
        release_call(&effect.call_id);
    }
    Some((
        effect.call_id.clone(),
        effect_result_frame(&effect.call_id, ok, error.as_deref()),
    ))
}

use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use tokio::sync::{broadcast, Mutex};
use uuid::Uuid;

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::event_bus::{DomainEvent, EventHandler, SubscriptionHandle};
use crate::core::socketio::{SubagentProgressDetail, WebChannelEvent};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::agent::profiles::{AgentProfile, AgentProfileStore, DEFAULT_PROFILE_ID};
use crate::openhuman::agent::Agent;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;
use crate::openhuman::prompt_injection::{
    enforce_prompt_input, PromptEnforcementAction, PromptEnforcementContext,
};
use crate::openhuman::threads::turn_state::{TurnStateMirror, TurnStateStore};
use crate::rpc::RpcOutcome;

use super::presentation;

static EVENT_BUS: Lazy<broadcast::Sender<WebChannelEvent>> = Lazy::new(|| {
    let (tx, _rx) = broadcast::channel(512);
    tx
});

pub fn subscribe_web_channel_events() -> broadcast::Receiver<WebChannelEvent> {
    EVENT_BUS.subscribe()
}

pub fn publish_web_channel_event(event: WebChannelEvent) {
    let _ = EVENT_BUS.send(event);
}

static APPROVAL_SURFACE_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Bridge a parked `ApprovalGate` request onto the web channel. When the gate
/// publishes `ApprovalRequested` carrying a chat thread/client (set via the
/// per-turn `ApprovalChatContext`), surface the "run X? (yes/no)" question as an
/// `approval_request` event on that thread so the user can answer in chat.
/// Idempotent. No-op for non-chat approvals (thread/client id absent).
pub fn register_approval_surface_subscriber() {
    if APPROVAL_SURFACE_HANDLE.get().is_some() {
        return;
    }
    match crate::core::event_bus::subscribe_global(Arc::new(ApprovalSurfaceSubscriber)) {
        Some(handle) => {
            let _ = APPROVAL_SURFACE_HANDLE.set(handle);
            log::info!(
                "[web-channel] approval-surface subscriber registered (domain=approval) — will bridge ApprovalRequested → approval_request socket event"
            );
        }
        None => {
            log::warn!(
                "[web-channel] failed to register approval-surface subscriber — bus not initialized"
            );
        }
    }
}

/// Handle for the artifact-surface subscriber. Set once on
/// [`register_artifact_surface_subscriber`]; subsequent calls no-op.
static ARTIFACT_SURFACE_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Bridge artifact lifecycle events onto the web channel.
/// `DomainEvent::ArtifactPending` / `ArtifactReady` / `ArtifactFailed`
/// (published by `artifacts::store::{create,finalize,fail}_artifact`)
/// carry the thread_id + client_id when the producing turn ran under an
/// `APPROVAL_CHAT_CONTEXT`. When present, fan out as an
/// `artifact_pending` / `artifact_ready` / `artifact_failed` socket
/// event so the frontend `chatRuntimeSlice` can upsert the snapshot and
/// the `ArtifactCard` can render in the message timeline:
///
/// - `artifact_pending` → render an in-progress "Generating…" card the
///   moment the producing tool dispatches (#3162).
/// - `artifact_ready` → swap the same card to a download surface when
///   the file lands (#2779).
/// - `artifact_failed` → swap to a retry-hint card on producer error.
///
/// The card is keyed on `artifact_id`, so the Pending → Ready/Failed
/// transition reuses the same surface instead of flickering a new one.
/// Idempotent. No-op for non-chat events (thread/client id absent).
pub fn register_artifact_surface_subscriber() {
    if ARTIFACT_SURFACE_HANDLE.get().is_some() {
        return;
    }
    match crate::core::event_bus::subscribe_global(Arc::new(ArtifactSurfaceSubscriber)) {
        Some(handle) => {
            let _ = ARTIFACT_SURFACE_HANDLE.set(handle);
            log::info!(
                "[web-channel] artifact-surface subscriber registered (domain=artifact) — will bridge ArtifactPending/Ready/Failed → artifact_pending/artifact_ready/artifact_failed socket events"
            );
        }
        None => {
            log::warn!(
                "[web-channel] failed to register artifact-surface subscriber — bus not initialized"
            );
        }
    }
}

struct ArtifactSurfaceSubscriber;

#[async_trait]
impl EventHandler for ArtifactSurfaceSubscriber {
    fn name(&self) -> &str {
        "channels::web::artifact_surface"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["artifact"])
    }

    async fn handle(&self, event: &DomainEvent) {
        match event {
            DomainEvent::ArtifactReady {
                artifact_id,
                kind,
                title,
                workspace_dir,
                path,
                size_bytes,
                thread_id,
                client_id,
            } => {
                let (Some(thread_id), Some(client_id)) = (thread_id, client_id) else {
                    log::debug!(
                        "[web-channel] artifact-surface skip ArtifactReady id={artifact_id}: no chat context"
                    );
                    return;
                };
                log::info!(
                    "[web-channel] artifact-surface emitting artifact_ready id={artifact_id} kind={kind} thread_id={thread_id} client_id={client_id}"
                );
                publish_web_channel_event(WebChannelEvent {
                    event: "artifact_ready".to_string(),
                    client_id: client_id.clone(),
                    thread_id: thread_id.clone(),
                    args: Some(serde_json::json!({
                        "artifact_id": artifact_id,
                        "kind": kind,
                        "title": title,
                        "workspace_dir": workspace_dir,
                        "path": path,
                        "size_bytes": size_bytes,
                    })),
                    ..Default::default()
                });
            }
            DomainEvent::ArtifactFailed {
                artifact_id,
                kind,
                title,
                workspace_dir,
                error,
                thread_id,
                client_id,
            } => {
                let (Some(thread_id), Some(client_id)) = (thread_id, client_id) else {
                    log::debug!(
                        "[web-channel] artifact-surface skip ArtifactFailed id={artifact_id}: no chat context"
                    );
                    return;
                };
                log::warn!(
                    "[web-channel] artifact-surface emitting artifact_failed id={artifact_id} kind={kind} thread_id={thread_id} client_id={client_id} error_len={}",
                    error.len()
                );
                publish_web_channel_event(WebChannelEvent {
                    event: "artifact_failed".to_string(),
                    client_id: client_id.clone(),
                    thread_id: thread_id.clone(),
                    args: Some(serde_json::json!({
                        "artifact_id": artifact_id,
                        "kind": kind,
                        "title": title,
                        "workspace_dir": workspace_dir,
                        "error": error,
                    })),
                    ..Default::default()
                });
            }
            DomainEvent::ArtifactPending {
                artifact_id,
                kind,
                title,
                workspace_dir,
                path,
                thread_id,
                client_id,
            } => {
                let (Some(thread_id), Some(client_id)) = (thread_id, client_id) else {
                    log::debug!(
                        "[web-channel] artifact-surface skip ArtifactPending id={artifact_id}: no chat context"
                    );
                    return;
                };
                log::info!(
                    "[web-channel] artifact-surface emitting artifact_pending id={artifact_id} kind={kind} thread_id={thread_id} client_id={client_id}"
                );
                publish_web_channel_event(WebChannelEvent {
                    event: "artifact_pending".to_string(),
                    client_id: client_id.clone(),
                    thread_id: thread_id.clone(),
                    args: Some(serde_json::json!({
                        "artifact_id": artifact_id,
                        "kind": kind,
                        "title": title,
                        "workspace_dir": workspace_dir,
                        "path": path,
                    })),
                    ..Default::default()
                });
            }
            _ => {}
        }
    }
}

struct ApprovalSurfaceSubscriber;

#[async_trait]
impl EventHandler for ApprovalSurfaceSubscriber {
    fn name(&self) -> &str {
        "channels::web::approval_surface"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["approval"])
    }

    async fn handle(&self, event: &DomainEvent) {
        if let DomainEvent::ApprovalRequested {
            request_id,
            tool_name,
            action_summary,
            args_redacted,
            thread_id,
            client_id,
            ..
        } = event
        {
            match (thread_id, client_id) {
                (Some(thread_id), Some(client_id)) => {
                    // Short, neutral description — the card renders the exact
                    // command/args (from `args` below) and has Approve/Deny
                    // buttons, so no "reply yes/no" instruction here.
                    let question = format!("Run `{tool_name}` — {action_summary}");
                    log::info!(
                        "[web-channel] approval-surface emitting approval_request request_id={request_id} thread_id={thread_id} client_id={client_id} tool={tool_name}"
                    );
                    publish_web_channel_event(WebChannelEvent {
                        event: "approval_request".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: Some(tool_name.clone()),
                        message: Some(question),
                        // The exact (redacted) command/args being requested, so
                        // the card can show precisely what will run.
                        args: Some(args_redacted.clone()),
                        ..Default::default()
                    });
                }
                _ => {
                    log::warn!(
                        "[web-channel] approval-surface received ApprovalRequested request_id={request_id} tool={tool_name} but thread_id/client_id absent (thread={}, client={}) — NOT surfacing",
                        thread_id.is_some(),
                        client_id.is_some()
                    );
                }
            }
        }
    }
}

/// All inputs that the cached `SessionEntry`'s `Agent` was built from,
/// captured at build time. The cache-hit predicate is a single
/// `entry.fingerprint == current_fingerprint` comparison — pulling the
/// fields into a named struct (instead of inlining four `&&`s) makes
/// the predicate testable in isolation and makes "what invalidates the
/// cache?" answerable in one place.
///
/// Adding a new dimension that should force a rebuild = add a field
/// here and populate it both at insert time and at the call-site
/// fingerprint construction.
#[derive(PartialEq, Debug, Clone)]
struct SessionCacheFingerprint {
    /// Per-message `model_override` (clients can override the model
    /// for an individual chat call).
    model_override: Option<String>,
    /// Per-message `temperature` override (same channel as
    /// `model_override`).
    temperature: Option<f64>,
    /// Which agent definition was used to build `agent`. Tracked so cache
    /// invalidation can detect when the target changes between turns.
    target_agent_id: String,
    /// Bound provider string at build time for the selected workload
    /// role (`chat`, `reasoning`, `agentic`, `coding`, `summarization`).
    ///
    /// Web-chat sessions cache a fully constructed `Agent`, which in
    /// turn holds a concrete provider instance chosen up front by the
    /// session builder. If the bound provider string changes in
    /// Settings, the cache must invalidate so the next turn rebuilds
    /// against the updated provider rather than silently reusing the
    /// stale instance.
    provider_binding: String,
    /// Signature of the autonomy/access config (`[autonomy]`) at build time.
    /// The cached `Agent` holds tools that each captured a `SecurityPolicy`
    /// snapshot at construction, so a change to the agent-access tier
    /// (`config.update_autonomy_settings` → Settings → Agent access) must
    /// invalidate the cache — otherwise the next turn silently reuses tools
    /// gated by the OLD policy and the setting appears to do nothing. Derived
    /// from the on-disk autonomy block (read fresh each turn), so it flips the
    /// moment a new tier is saved.
    autonomy_signature: String,
}

struct SessionEntry {
    agent: Agent,
    fingerprint: SessionCacheFingerprint,
}

/// Deterministic signature of the autonomy/access config for the session cache
/// fingerprint. Serializing the whole `[autonomy]` block (serde emits fields in
/// stable declaration order) captures every knob that feeds `SecurityPolicy` —
/// `level`, `workspace_only`, `trusted_roots`, `allow_tool_install`,
/// `allowed_commands`, … — so saving any agent-access change flips the
/// signature and forces a rebuild. On the practically-impossible serialize
/// error we return an empty string, which just means "treat as changed".
fn autonomy_signature(config: &Config) -> String {
    serde_json::to_string(&config.autonomy).unwrap_or_default()
}

/// Decide which agent definition this turn should run with.
///
/// All new chat turns route to the `orchestrator` agent directly.
/// The welcome agent has been removed; the Joyride walkthrough in the
/// frontend handles onboarding UI instead.
fn pick_target_agent_id(_config: &Config, profile: &AgentProfile) -> String {
    if profile.id == DEFAULT_PROFILE_ID {
        "orchestrator".to_string()
    } else {
        profile.agent_id.clone()
    }
}

#[derive(Debug)]
struct InFlightEntry {
    request_id: String,
    handle: tokio::task::JoinHandle<()>,
    run_queue: Arc<crate::openhuman::agent::harness::run_queue::RunQueue>,
}

#[derive(Debug, Clone)]
struct WebChatTaskResult {
    full_response: String,
    citations: Vec<crate::openhuman::agent::memory_loader::MemoryCitation>,
}

static THREAD_SESSIONS: Lazy<Mutex<HashMap<String, SessionEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

static IN_FLIGHT: Lazy<Mutex<HashMap<String, InFlightEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
#[cfg(any(test, debug_assertions))]
static TEST_FORCED_RUN_CHAT_TASK_ERROR: Lazy<Mutex<Option<String>>> =
    Lazy::new(|| Mutex::new(None));
/// Key for the per-thread runtime maps (`THREAD_SESSIONS`, `IN_FLIGHT`).
///
/// Keyed by `thread_id` ALONE — the stable, persistent identity of a
/// conversation — NOT by the Socket.IO `client_id`, which is regenerated on
/// every reconnect. Keying these maps by `client_id` previously orphaned a
/// thread's cached session (conversation amnesia) and its in-flight task handle
/// (Cancel became a no-op) whenever the socket reconnected with a new id. Event
/// delivery still routes by `client_id` (the live socket); only the
/// thread-owned runtime state keys off `thread_id`.
fn key_for(thread_id: &str) -> String {
    thread_id.to_string()
}

fn event_session_id_for(client_id: &str, thread_id: &str) -> String {
    json!({
        "client_id": client_id,
        "thread_id": thread_id,
    })
    .to_string()
}

#[path = "web_errors.rs"]
mod web_errors;
pub(crate) use web_errors::{
    classify_inference_error, inference_budget_exceeded_user_message,
    is_inference_budget_exceeded_error,
};
#[cfg(any(test, debug_assertions))]
#[allow(unused_imports)]
pub(crate) use web_errors::{
    extract_provider_error_detail, extract_provider_name, generic_inference_error_user_message,
    is_action_budget_exhausted, is_fallback_chain_exhausted, is_non_retryable_rate_limit_text,
    parse_retry_after_secs_from_str, retry_after_hint, with_provider_detail, ClassifiedError,
};

#[cfg(any(test, debug_assertions))]
pub mod test_support {
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ClassifiedErrorSnapshot {
        pub error_type: &'static str,
        pub message: String,
        pub source: &'static str,
        pub retryable: bool,
        pub retry_after_ms: Option<u64>,
        pub provider: Option<String>,
        pub fallback_available: Option<bool>,
    }

    pub fn classify_error_for_test(err: &str) -> ClassifiedErrorSnapshot {
        let classified = super::classify_inference_error(err);
        ClassifiedErrorSnapshot {
            error_type: classified.error_type,
            message: classified.message,
            source: classified.source,
            retryable: classified.retryable,
            retry_after_ms: classified.retry_after_ms,
            provider: classified.provider,
            fallback_available: classified.fallback_available,
        }
    }

    pub fn extracted_provider_detail_for_test(err: &str) -> Option<String> {
        super::extract_provider_error_detail(err)
    }

    pub fn retry_after_secs_for_test(err: &str) -> Option<u64> {
        super::parse_retry_after_secs_from_str(err)
    }

    pub fn is_non_retryable_rate_limit_for_test(lower: &str) -> bool {
        super::is_non_retryable_rate_limit_text(lower)
    }

    pub fn key_for_test(thread_id: &str) -> String {
        super::key_for(thread_id)
    }

    pub fn event_session_id_for_test(client_id: &str, thread_id: &str) -> String {
        super::event_session_id_for(client_id, thread_id)
    }

    pub async fn set_forced_run_chat_task_error_for_test(message: Option<&str>) {
        super::set_test_forced_run_chat_task_error(message).await;
    }
}

fn prompt_guard_user_message(action: PromptEnforcementAction) -> &'static str {
    match action {
        PromptEnforcementAction::Allow => "Message accepted.",
        PromptEnforcementAction::Blocked => {
            "Your message was blocked by a security policy. Please rephrase and remove instruction-override or secret-exfiltration requests."
        }
        PromptEnforcementAction::ReviewBlocked => {
            "Your message was flagged for security review and was not processed. Please rephrase the request in a direct, task-focused way."
        }
    }
}

#[cfg(any(test, debug_assertions))]
pub(super) async fn set_test_forced_run_chat_task_error(message: Option<&str>) {
    let mut slot = TEST_FORCED_RUN_CHAT_TASK_ERROR.lock().await;
    *slot = message.map(str::to_string);
}

pub async fn start_chat(
    client_id: &str,
    thread_id: &str,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    profile_id: Option<String>,
    locale: Option<String>,
    queue_mode: Option<String>,
) -> Result<String, String> {
    let client_id = client_id.trim().to_string();
    let thread_id = thread_id.trim().to_string();
    let message = message.trim().to_string();

    if client_id.is_empty() {
        return Err("client_id is required".to_string());
    }
    if thread_id.is_empty() {
        return Err("thread_id is required".to_string());
    }
    if message.is_empty() {
        return Err("message is required".to_string());
    }

    let request_id = Uuid::new_v4().to_string();
    let prompt_decision = enforce_prompt_input(
        &message,
        PromptEnforcementContext {
            source: "channels.providers.web.start_chat",
            request_id: Some(&request_id),
            user_id: Some(&client_id),
            session_id: Some(&thread_id),
        },
    );
    if !matches!(prompt_decision.action, PromptEnforcementAction::Allow) {
        log::warn!(
            "[web-channel] prompt rejected client_id={} thread_id={} request_id={} action={} score={:.2} reasons={} hash={} chars={}",
            client_id,
            thread_id,
            request_id,
            match prompt_decision.action {
                PromptEnforcementAction::Allow => "allow",
                PromptEnforcementAction::Blocked => "block",
                PromptEnforcementAction::ReviewBlocked => "review_blocked",
            },
            prompt_decision.score,
            prompt_decision
                .reasons
                .iter()
                .map(|r| r.code.as_str())
                .collect::<Vec<_>>()
                .join(","),
            prompt_decision.prompt_hash,
            prompt_decision.prompt_chars,
        );
        return Err(prompt_guard_user_message(prompt_decision.action).to_string());
    }

    // Chat-native approval: if this thread has a parked approval and the message
    // is a yes/no reply, route it to the gate (resuming the parked turn) rather
    // than starting a new turn — which would cancel the parked approval. Any
    // other text falls through to the normal path below, which cancels the
    // in-flight turn and dispatches the message fresh (the intended "redirect").
    if let Some(gate) = crate::openhuman::approval::ApprovalGate::try_global() {
        if let Some(request_id) = gate.pending_for_thread(&thread_id) {
            if let Some(decision) = crate::openhuman::approval::parse_approval_reply(&message) {
                match gate.decide(&request_id, decision) {
                    Ok(Some(_)) => {
                        log::info!(
                            "[web-channel] routed chat reply to approval gate thread_id={} request_id={} decision={}",
                            thread_id,
                            request_id,
                            decision.as_str()
                        );
                        return Ok(request_id);
                    }
                    Ok(None) => {
                        // `decide` returns `Ok(None)` when the request is already
                        // gone / already decided — the parked turn was NOT resumed
                        // by this call. Don't ACK it as applied; fall through so the
                        // reply is dispatched as a fresh turn.
                        log::warn!(
                            "[web-channel] approval reply targeted a non-pending/already-decided request thread_id={} request_id={} decision={} — dispatching as fresh turn",
                            thread_id,
                            request_id,
                            decision.as_str()
                        );
                    }
                    Err(err) => {
                        // Don't claim success: the parked turn is still waiting on
                        // its oneshot. Log and fall through so the reply is
                        // dispatched as a fresh turn rather than silently dropped
                        // (the stale parked request will TTL out).
                        log::warn!(
                            "[web-channel] failed to route chat reply to approval gate thread_id={} request_id={} decision={} err={}",
                            thread_id,
                            request_id,
                            decision.as_str(),
                            err
                        );
                    }
                }
            }
        }
    }

    let map_key = key_for(&thread_id);

    let parsed_mode = match queue_mode.as_deref() {
        Some("steer") => crate::openhuman::agent::harness::run_queue::QueueMode::Steer,
        Some("followup") => crate::openhuman::agent::harness::run_queue::QueueMode::Followup,
        Some("collect") => crate::openhuman::agent::harness::run_queue::QueueMode::Collect,
        _ => crate::openhuman::agent::harness::run_queue::QueueMode::Interrupt,
    };

    // Non-interrupt modes: push into the running turn's queue and return.
    if !matches!(
        parsed_mode,
        crate::openhuman::agent::harness::run_queue::QueueMode::Interrupt
    ) {
        let in_flight = IN_FLIGHT.lock().await;
        if let Some(existing) = in_flight.get(&map_key) {
            let queued_msg = crate::openhuman::agent::harness::run_queue::QueuedMessage {
                text: message.clone(),
                mode: parsed_mode,
                client_id: client_id.clone(),
                thread_id: thread_id.clone(),
                queued_at_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                model_override: model_override.clone(),
                temperature,
                profile_id: profile_id.clone(),
                locale: locale.clone(),
            };
            existing.run_queue.push(queued_msg).await;
            let status = existing.run_queue.status().await;
            log::info!(
                "[web-channel] queued {} message thread_id={} request_id={} queue_depth={}",
                parsed_mode,
                thread_id,
                request_id,
                status.total
            );
            crate::core::event_bus::publish_global(DomainEvent::RunQueueMessageQueued {
                thread_id: thread_id.clone(),
                mode: parsed_mode.to_string(),
                queue_depth: status.total,
            });
            return Ok(json!({
                "queued": true,
                "queue_mode": parsed_mode.to_string(),
                "client_id": client_id,
                "thread_id": thread_id,
                "request_id": request_id,
                "queue_depth": status.total,
            })
            .to_string());
        }
        // No in-flight turn — fall through to start a fresh turn.
        log::info!(
            "[web-channel] no in-flight turn for {} mode thread_id={} — starting fresh",
            parsed_mode,
            thread_id
        );
    }

    {
        let mut in_flight = IN_FLIGHT.lock().await;
        if let Some(existing) = in_flight.remove(&map_key) {
            existing.handle.abort();
            log::info!(
                "[web-channel] interrupted in-flight turn thread_id={} cancelled_request_id={}",
                thread_id,
                existing.request_id
            );
            crate::core::event_bus::publish_global(DomainEvent::RunQueueInterrupted {
                thread_id: thread_id.clone(),
                cancelled_request_id: existing.request_id.clone(),
            });
            publish_web_channel_event(WebChannelEvent {
                event: "chat_error".to_string(),
                client_id: client_id.clone(),
                thread_id: thread_id.clone(),
                request_id: existing.request_id,
                full_response: None,
                message: Some("Cancelled by newer request".to_string()),
                error_type: Some("cancelled".to_string()),
                error_source: None,
                error_retryable: None,
                error_retry_after_ms: None,
                error_provider: None,
                error_fallback_available: None,
                tool_name: None,
                skill_id: None,
                args: None,
                output: None,
                success: None,
                round: None,
                reaction_emoji: None,
                segment_index: None,
                segment_total: None,
                delta: None,
                delta_kind: None,
                tool_call_id: None,
                citations: None,
                subagent: None,
                task_board: None,
            });
        }
    }

    let turn_run_queue = crate::openhuman::agent::harness::run_queue::RunQueue::new();
    let turn_run_queue_task = turn_run_queue.clone();

    let client_id_task = client_id.clone();
    let thread_id_task = thread_id.clone();
    let request_id_task = request_id.clone();
    let map_key_task = map_key.clone();

    let user_message = message.clone();
    let handle = tokio::spawn(async move {
        // Scope the per-turn approval chat context so a parked `ApprovalGate`
        // request (raised deep in the tool loop, which runs inline in this same
        // task) carries the thread/client id — letting a yes/no chat reply be
        // routed back to `approval_decide`. No sub-task is spawned between here
        // and `intercept`, so the task-local propagates.
        let approval_ctx = crate::openhuman::approval::ApprovalChatContext {
            thread_id: thread_id_task.clone(),
            client_id: client_id_task.clone(),
        };
        // Scope the matching `AgentTurnOrigin::WebChat` alongside the chat
        // context so the approval gate's origin-aware decision tree sees a
        // web-routable turn. Both task-locals must wrap the same future —
        // tokio task-locals do not cross `tokio::spawn`, and `intercept`
        // runs inline within this task.
        let origin = crate::openhuman::agent::turn_origin::AgentTurnOrigin::WebChat {
            thread_id: thread_id_task.clone(),
            client_id: client_id_task.clone(),
        };
        let result = crate::openhuman::agent::turn_origin::with_origin(
            origin,
            crate::openhuman::approval::APPROVAL_CHAT_CONTEXT.scope(
                approval_ctx,
                run_chat_task(
                    &client_id_task,
                    &thread_id_task,
                    &request_id_task,
                    &user_message,
                    model_override,
                    temperature,
                    profile_id,
                    locale,
                    turn_run_queue_task,
                ),
            ),
        )
        .await;

        match result {
            Ok(chat_result) => {
                // ── Presentation layer (local model, fire-and-forget) ─────
                // Segment the response into human-readable bubbles and
                // decide whether to react — both run via local Ollama if
                // available, zero cloud cost.
                presentation::deliver_response(
                    &client_id_task,
                    &thread_id_task,
                    &request_id_task,
                    &chat_result.full_response,
                    &user_message,
                    &chat_result.citations,
                )
                .await;
            }
            Err(err) => {
                log::warn!(
                    "[web-channel] run_chat_task failed client_id={} thread_id={} request_id={} error={}",
                    client_id_task,
                    thread_id_task,
                    request_id_task,
                    err
                );
                let detailed = format!(
                    "run_chat_task failed client_id={} thread_id={} request_id={} error={}",
                    client_id_task, thread_id_task, request_id_task, err
                );
                let classified = classify_inference_error(&err);
                let classified_type = classified.error_type;
                let classified_type_string = classified_type.to_string();
                // Max-tool-iterations cap is a deterministic agent-state
                // outcome surfaced to the user via the existing
                // `WebChannelEvent::chat_error` event below. Skip the
                // Sentry funnel entirely for that variant
                // (OPENHUMAN-TAURI-98). Substring match is required here
                // because the typed `AgentError` was flattened to a
                // `String` at the native-bus boundary.
                //
                // Other errors flow through `report_error_or_expected`
                // so transport-level transient failures (DNS/TCP/TLS
                // handshake, ISP blocks — OPENHUMAN-TAURI-32 for the RU
                // user who couldn't reach api.tinyhumans.ai at all) get
                // logged as warn-level breadcrumbs instead of error
                // events. Sentry has no signal to act on those — no
                // status, no trace, no payload — and every retry
                // exhaustion produces another noisy event.
                if crate::openhuman::agent::error::is_max_iterations_error(&detailed) {
                    log::info!(
                        target: "web_channel",
                        "[web_channel.run_chat_task] suppressed Sentry emission for max-iteration \
                         cap client_id={} thread_id={} request_id={} error_type={} message={}",
                        client_id_task,
                        thread_id_task,
                        request_id_task,
                        classified_type,
                        detailed
                    );
                } else {
                    crate::core::observability::report_error_or_expected(
                        detailed.as_str(),
                        "web_channel",
                        "run_chat_task",
                        &[
                            ("channel", "web"),
                            ("error_type", classified_type),
                            ("thread_id", thread_id_task.as_str()),
                            ("request_id", request_id_task.as_str()),
                        ],
                    );
                }
                publish_web_channel_event(WebChannelEvent {
                    event: "chat_error".to_string(),
                    client_id: client_id_task.clone(),
                    thread_id: thread_id_task.clone(),
                    request_id: request_id_task.clone(),
                    full_response: None,
                    message: Some(classified.message),
                    error_type: Some(classified_type_string),
                    error_source: Some(classified.source.to_string()),
                    error_retryable: Some(classified.retryable),
                    error_retry_after_ms: classified.retry_after_ms,
                    error_provider: classified.provider,
                    error_fallback_available: classified.fallback_available,
                    tool_name: None,
                    skill_id: None,
                    args: None,
                    output: None,
                    success: None,
                    round: None,
                    reaction_emoji: None,
                    segment_index: None,
                    segment_total: None,
                    delta: None,
                    delta_kind: None,
                    tool_call_id: None,
                    citations: None,
                    subagent: None,
                    task_board: None,
                });
            }
        }

        // Drain followup messages queued during this turn.
        let followups = {
            let mut in_flight = IN_FLIGHT.lock().await;
            let followups = if let Some(current) = in_flight.get(&map_key_task) {
                if current.request_id == request_id_task {
                    let fups = current.run_queue.drain_followups().await;
                    in_flight.remove(&map_key_task);
                    fups
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            followups
        };
        if !followups.is_empty() {
            log::info!(
                "[web-channel] dispatching {} followup(s) thread_id={}",
                followups.len(),
                thread_id_task
            );
            crate::core::event_bus::publish_global(
                crate::core::event_bus::DomainEvent::RunQueueFollowupDispatched {
                    thread_id: thread_id_task.clone(),
                    followup_count: followups.len(),
                },
            );
            // Dispatch each followup as a fresh turn on a new task to avoid
            // Send issues with the nested async closure.
            dispatch_followups(followups);
        }
    });

    {
        let mut in_flight = IN_FLIGHT.lock().await;
        in_flight.insert(
            map_key,
            InFlightEntry {
                request_id: request_id.clone(),
                handle,
                run_queue: turn_run_queue,
            },
        );
    }

    Ok(request_id)
}

fn dispatch_followups(followups: Vec<crate::openhuman::agent::harness::run_queue::QueuedMessage>) {
    for fup in followups {
        tokio::spawn(async move {
            if let Err(err) = start_chat(
                &fup.client_id,
                &fup.thread_id,
                &fup.text,
                fup.model_override,
                fup.temperature,
                fup.profile_id,
                fup.locale,
                Some("followup".to_string()),
            )
            .await
            {
                log::warn!(
                    "[web-channel] failed to dispatch followup thread_id={} err={}",
                    fup.thread_id,
                    err
                );
            }
        });
    }
}

/// Invalidate all cached agent sessions for the given thread ID.
/// Called when a thread is deleted so stale sessions don't leak
/// into reused thread IDs.
pub async fn invalidate_thread_sessions(thread_id: &str) {
    let mut sessions = THREAD_SESSIONS.lock().await;
    let keys_to_remove: Vec<String> = sessions
        .keys()
        .filter(|k| k.as_str() == thread_id || k.ends_with(&format!("::{thread_id}")))
        .cloned()
        .collect();
    for key in &keys_to_remove {
        sessions.remove(key);
    }
    if !keys_to_remove.is_empty() {
        log::debug!(
            "[web-channel] invalidated {} cached session(s) for thread_id={}",
            keys_to_remove.len(),
            thread_id
        );
    }
}

/// Snapshot the IN_FLIGHT map for the test-support introspection RPC.
///
/// Returned as `(map_key, request_id)` pairs. Not intended for any
/// production caller — release builds reach this via the bearer-gated
/// `/rpc` endpoint only, and the per-launch token file is debug-only.
pub async fn in_flight_entries_for_test() -> Vec<(String, String)> {
    let guard = IN_FLIGHT.lock().await;
    guard
        .iter()
        .map(|(k, v)| (k.clone(), v.request_id.clone()))
        .collect()
}

pub async fn cancel_chat(client_id: &str, thread_id: &str) -> Result<Option<String>, String> {
    let client_id = client_id.trim();
    let thread_id = thread_id.trim();

    if client_id.is_empty() {
        return Err("client_id is required".to_string());
    }
    if thread_id.is_empty() {
        return Err("thread_id is required".to_string());
    }

    let map_key = key_for(thread_id);
    let mut removed_request_id: Option<String> = None;

    {
        let mut in_flight = IN_FLIGHT.lock().await;
        if let Some(existing) = in_flight.remove(&map_key) {
            removed_request_id = Some(existing.request_id.clone());
            existing.handle.abort();
        }
    }

    if let Some(request_id) = removed_request_id.clone() {
        publish_web_channel_event(WebChannelEvent {
            event: "chat_error".to_string(),
            client_id: client_id.to_string(),
            thread_id: thread_id.to_string(),
            request_id,
            full_response: None,
            message: Some("Cancelled".to_string()),
            error_type: Some("cancelled".to_string()),
            error_source: None,
            error_retryable: None,
            error_retry_after_ms: None,
            error_provider: None,
            error_fallback_available: None,
            tool_name: None,
            skill_id: None,
            args: None,
            output: None,
            success: None,
            round: None,
            reaction_emoji: None,
            segment_index: None,
            segment_total: None,
            delta: None,
            delta_kind: None,
            tool_call_id: None,
            citations: None,
            subagent: None,
            task_board: None,
        });
    }

    Ok(removed_request_id)
}

async fn run_chat_task(
    client_id: &str,
    thread_id: &str,
    request_id: &str,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    profile_id: Option<String>,
    locale: Option<String>,
    run_queue: Arc<crate::openhuman::agent::harness::run_queue::RunQueue>,
) -> Result<WebChatTaskResult, String> {
    #[cfg(any(test, debug_assertions))]
    {
        let mut slot = TEST_FORCED_RUN_CHAT_TASK_ERROR.lock().await;
        if let Some(forced) = slot.take() {
            log::debug!(
                "[web-channel][test] forced run_chat_task failure client_id={} thread_id={} request_id={}",
                client_id,
                thread_id,
                request_id
            );
            return Err(forced);
        }
    }

    let config = config_rpc::load_config_with_timeout().await?;
    let (_profiles_state, profile) =
        AgentProfileStore::new(config.workspace_dir.clone()).resolve(profile_id.as_deref())?;
    let map_key = key_for(thread_id);
    let model_override = normalize_model_override(profile.model_override.clone())
        .or_else(|| normalize_model_override(model_override));
    let temperature = profile.temperature.or(temperature);
    // Compute the routing decision up front so the cache lookup can
    // detect when it has changed. This also keeps non-default profile
    // switches from reusing a cached agent built for another target.
    let target_agent_id = pick_target_agent_id(&config, &profile);
    let provider_role = provider_role_for_model_override(model_override.as_deref());
    let current_fp = SessionCacheFingerprint {
        model_override: model_override.clone(),
        temperature,
        target_agent_id: target_agent_id.clone(),
        provider_binding: crate::openhuman::inference::provider::provider_for_role(
            provider_role,
            &config,
        ),
        autonomy_signature: autonomy_signature(&config),
    };

    let prior = {
        let mut sessions = THREAD_SESSIONS.lock().await;
        sessions.remove(&map_key)
    };

    let (mut agent, was_built_fresh) = match prior {
        Some(entry) if entry.fingerprint == current_fp => {
            log::info!(
                "[web-channel] reusing cached session agent id={} for client={} thread={}",
                target_agent_id,
                client_id,
                thread_id
            );
            (entry.agent, false)
        }
        Some(prior_entry) => {
            log::info!(
                "[web-channel] cache miss — rebuilding session agent \
                 (was id={}, now id={}; prior_provider_binding={}, now={}) \
                 for client={} thread={}",
                prior_entry.fingerprint.target_agent_id,
                target_agent_id,
                prior_entry.fingerprint.provider_binding,
                current_fp.provider_binding,
                client_id,
                thread_id
            );
            (
                build_session_agent(
                    &config,
                    client_id,
                    thread_id,
                    &target_agent_id,
                    &profile,
                    model_override.clone(),
                    temperature,
                    locale.as_deref(),
                )?,
                true,
            )
        }
        None => (
            build_session_agent(
                &config,
                client_id,
                thread_id,
                &target_agent_id,
                &profile,
                model_override.clone(),
                temperature,
                locale.as_deref(),
            )?,
            true,
        ),
    };

    // Cold-boot resume from the conversation JSONL.
    //
    // The agent's `try_load_session_transcript` mechanism only fires
    // when a transcript file matches `agent_definition_name` — it
    // misses on cold boot if the previous process wrote transcripts
    // under a different name (the `set_agent_definition_name` /
    // `session_key` rename bug fixed in this PR). The conversation
    // JSONL store is the authoritative per-thread message log either
    // way, so seed from it whenever we just built a fresh agent. The
    // method is a no-op if the agent already has a cached transcript
    // or non-empty history, so this is cheap on the warm path too.
    if was_built_fresh {
        match crate::openhuman::memory_conversations::get_messages(
            config.workspace_dir.clone(),
            thread_id,
        ) {
            Ok(prior_messages) if !prior_messages.is_empty() => {
                let pairs: Vec<(String, String)> = prior_messages
                    .into_iter()
                    .map(|m| (m.sender, m.content))
                    .collect();
                if let Err(err) = agent.seed_resume_from_messages(pairs, message) {
                    log::warn!(
                        "[web-channel] failed to seed agent resume from conversation log \
                         thread={} err={}",
                        thread_id,
                        err
                    );
                }
            }
            Ok(_) => {
                log::debug!(
                    "[web-channel] no prior messages to seed for thread={} — first turn",
                    thread_id
                );
            }
            Err(err) => {
                log::warn!(
                    "[web-channel] failed to read conversation log for resume thread={} err={}",
                    thread_id,
                    err
                );
            }
        }
    }

    // Wire up a real-time progress channel so tool calls, iterations,
    // and sub-agent events are emitted to the web channel as they happen
    // (instead of retroactively after the loop finishes).
    let (progress_tx, progress_rx) = tokio::sync::mpsc::channel(64);
    agent.set_on_progress(Some(progress_tx));
    agent.set_run_queue(Some(run_queue));
    let turn_state_store = TurnStateStore::new(config.workspace_dir.clone());
    spawn_progress_bridge(
        progress_rx,
        client_id.to_string(),
        thread_id.to_string(),
        request_id.to_string(),
        turn_state_store,
    );

    // Make `thread_id` ambient for any outbound provider call inside
    // the agent loop. The OpenAI-compatible provider reads it via
    // `thread_context::current_thread_id()` and forwards it on
    // `/openai/v1/chat/completions` so the backend can group
    // InferenceLog entries and reuse the KV cache for this thread.
    let result = match crate::openhuman::inference::provider::thread_context::with_thread_id(
        thread_id.to_string(),
        agent.run_single(message),
    )
    .await
    {
        Ok(response) => {
            let citations = agent.take_last_turn_citations();
            Ok(WebChatTaskResult {
                full_response: response,
                citations,
            })
        }
        Err(err) => {
            let err_message = err.to_string();
            if is_inference_budget_exceeded_error(&err_message) {
                log::warn!(
                    "[web-channel] inference budget exhausted for client={} thread={} request_id={} error_category=budget_exhausted",
                    client_id,
                    thread_id,
                    request_id
                );
                Ok(WebChatTaskResult {
                    full_response: inference_budget_exceeded_user_message().to_string(),
                    citations: Vec::new(),
                })
            } else {
                Err(err_message)
            }
        }
    };

    // Clear the sender so it doesn't hold the channel open across sessions.
    agent.set_on_progress(None);

    {
        let mut sessions = THREAD_SESSIONS.lock().await;
        sessions.insert(
            map_key,
            SessionEntry {
                agent,
                fingerprint: current_fp,
            },
        );
    }

    result
}

/// Spawn a background task that reads [`AgentProgress`] events from the
/// agent turn loop and translates them into [`WebChannelEvent`]s tagged
/// with the correct client/thread/request IDs. The task runs until the
/// sender is dropped (i.e. when the agent turn finishes).
fn spawn_progress_bridge(
    mut rx: tokio::sync::mpsc::Receiver<crate::openhuman::agent::progress::AgentProgress>,
    client_id: String,
    thread_id: String,
    request_id: String,
    turn_state_store: TurnStateStore,
) {
    use crate::openhuman::agent::progress::AgentProgress;

    tokio::spawn(async move {
        log::debug!(
            "[web_channel][bridge] spawned client_id={} thread_id={} request_id={}",
            client_id,
            thread_id,
            request_id,
        );
        let mut round: u32 = 0;
        let mut events_seen: u64 = 0;
        let mut turn_state =
            TurnStateMirror::new(turn_state_store, thread_id.clone(), request_id.clone());
        while let Some(event) = rx.recv().await {
            events_seen += 1;
            turn_state.observe(&event);
            // Per-variant trace so branch decisions are visible in
            // terminal output when correlating progress over Socket.IO.
            // Kept at trace-level for high-volume deltas and debug for
            // lifecycle transitions.
            match &event {
                AgentProgress::TextDelta { delta, iteration } => {
                    log::trace!(
                        "[web_channel][bridge] text_delta round={} chars={} request_id={}",
                        iteration,
                        delta.len(),
                        request_id,
                    );
                }
                AgentProgress::ThinkingDelta { delta, iteration } => {
                    log::trace!(
                        "[web_channel][bridge] thinking_delta round={} chars={} request_id={}",
                        iteration,
                        delta.len(),
                        request_id,
                    );
                }
                AgentProgress::ToolCallArgsDelta {
                    call_id,
                    tool_name,
                    delta,
                    iteration,
                } => {
                    log::trace!(
                        "[web_channel][bridge] tool_args_delta round={} tool={} call_id={} chars={} request_id={}",
                        iteration,
                        tool_name,
                        call_id,
                        delta.len(),
                        request_id,
                    );
                }
                AgentProgress::ToolCallStarted {
                    call_id,
                    tool_name,
                    iteration,
                    ..
                } => {
                    log::debug!(
                        "[web_channel][bridge] tool_call round={} tool={} call_id={} request_id={}",
                        iteration,
                        tool_name,
                        call_id,
                        request_id,
                    );
                }
                AgentProgress::ToolCallCompleted {
                    call_id,
                    tool_name,
                    success,
                    iteration,
                    ..
                } => {
                    log::debug!(
                        "[web_channel][bridge] tool_result round={} tool={} call_id={} success={} request_id={}",
                        iteration,
                        tool_name,
                        call_id,
                        success,
                        request_id,
                    );
                }
                AgentProgress::SubagentFailed {
                    agent_id, error, ..
                } => {
                    log::warn!(
                        "[web_channel][bridge] subagent_failed agent_id={} err={} client_id={} thread_id={} request_id={}",
                        agent_id,
                        error,
                        client_id,
                        thread_id,
                        request_id,
                    );
                }
                other => {
                    log::debug!(
                        "[web_channel][bridge] lifecycle event={:?} request_id={}",
                        std::mem::discriminant(other),
                        request_id,
                    );
                }
            }
            match event {
                AgentProgress::TurnStarted => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "inference_start".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        full_response: None,
                        message: None,
                        error_type: None,
                        error_source: None,
                        error_retryable: None,
                        error_retry_after_ms: None,
                        error_provider: None,
                        error_fallback_available: None,
                        tool_name: None,
                        skill_id: None,
                        args: None,
                        output: None,
                        success: None,
                        round: None,
                        reaction_emoji: None,
                        segment_index: None,
                        segment_total: None,
                        delta: None,
                        delta_kind: None,
                        tool_call_id: None,
                        citations: None,
                        subagent: None,
                        task_board: None,
                    });
                }
                AgentProgress::IterationStarted {
                    iteration,
                    max_iterations,
                } => {
                    round = iteration;
                    publish_web_channel_event(WebChannelEvent {
                        event: "iteration_start".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        full_response: None,
                        message: Some(format!("Iteration {iteration}/{max_iterations}")),
                        error_type: None,
                        error_source: None,
                        error_retryable: None,
                        error_retry_after_ms: None,
                        error_provider: None,
                        error_fallback_available: None,
                        tool_name: None,
                        skill_id: None,
                        args: None,
                        output: None,
                        success: None,
                        round: Some(iteration),
                        reaction_emoji: None,
                        segment_index: None,
                        segment_total: None,
                        delta: None,
                        delta_kind: None,
                        tool_call_id: None,
                        citations: None,
                        subagent: None,
                        task_board: None,
                    });
                }
                AgentProgress::ToolCallStarted {
                    call_id,
                    tool_name,
                    arguments,
                    iteration,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "tool_call".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: Some(tool_name),
                        skill_id: Some("web_channel".to_string()),
                        args: Some(arguments),
                        round: Some(iteration),
                        tool_call_id: Some(call_id),
                        ..Default::default()
                    });
                }
                AgentProgress::ToolCallCompleted {
                    call_id,
                    tool_name,
                    success,
                    output_chars,
                    elapsed_ms,
                    iteration,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "tool_result".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: Some(tool_name),
                        skill_id: Some("web_channel".to_string()),
                        output: Some(
                            json!({"output_chars": output_chars, "elapsed_ms": elapsed_ms})
                                .to_string(),
                        ),
                        success: Some(success),
                        round: Some(iteration),
                        tool_call_id: Some(call_id),
                        ..Default::default()
                    });
                }
                AgentProgress::SubagentSpawned {
                    agent_id,
                    task_id,
                    mode,
                    dedicated_thread,
                    prompt_chars,
                    worker_thread_id,
                    display_name,
                } => {
                    let label = display_name.as_deref().unwrap_or(&agent_id);
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_spawned".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        message: Some(format!("Sub-agent '{label}' spawned")),
                        tool_name: Some(agent_id),
                        skill_id: Some(task_id),
                        round: Some(round),
                        subagent: Some(SubagentProgressDetail {
                            mode: Some(mode),
                            dedicated_thread: Some(dedicated_thread),
                            prompt_chars: Some(prompt_chars as u64),
                            worker_thread_id,
                            display_name,
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
                AgentProgress::SubagentCompleted {
                    agent_id,
                    task_id,
                    elapsed_ms,
                    iterations,
                    output_chars,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_completed".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        message: Some(format!(
                            "Sub-agent '{agent_id}' completed in {elapsed_ms}ms"
                        )),
                        tool_name: Some(agent_id),
                        skill_id: Some(task_id),
                        success: Some(true),
                        round: Some(round),
                        subagent: Some(SubagentProgressDetail {
                            elapsed_ms: Some(elapsed_ms),
                            iterations: Some(iterations),
                            output_chars: Some(output_chars as u64),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
                AgentProgress::SubagentFailed {
                    agent_id,
                    task_id,
                    error,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_failed".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        message: Some(error),
                        tool_name: Some(agent_id),
                        skill_id: Some(task_id),
                        success: Some(false),
                        round: Some(round),
                        ..Default::default()
                    });
                }
                AgentProgress::SubagentAwaitingUser {
                    agent_id,
                    task_id,
                    question,
                    worker_thread_id,
                } => {
                    log::debug!(
                        "[web_channel][bridge] subagent_awaiting_user agent_id={} task_id={} client_id={} thread_id={} request_id={}",
                        agent_id,
                        task_id,
                        client_id,
                        thread_id,
                        request_id,
                    );
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_awaiting_user".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        message: Some(question),
                        tool_name: Some(agent_id),
                        skill_id: Some(task_id),
                        success: Some(true),
                        round: Some(round),
                        subagent: Some(SubagentProgressDetail {
                            worker_thread_id,
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
                AgentProgress::SubagentIterationStarted {
                    agent_id,
                    task_id,
                    iteration,
                    max_iterations,
                    extended_policy,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_iteration_start".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        message: Some(if extended_policy {
                            format!("Sub-agent '{agent_id}' step {iteration}")
                        } else {
                            format!("Sub-agent '{agent_id}' iteration {iteration}/{max_iterations}")
                        }),
                        tool_name: Some(agent_id),
                        skill_id: Some(task_id),
                        round: Some(round),
                        subagent: Some(SubagentProgressDetail {
                            child_iteration: Some(iteration),
                            child_max_iterations: if extended_policy {
                                None
                            } else {
                                Some(max_iterations)
                            },
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
                AgentProgress::SubagentToolCallStarted {
                    agent_id,
                    task_id,
                    call_id,
                    tool_name,
                    iteration,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_tool_call".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: Some(tool_name),
                        skill_id: Some(task_id.clone()),
                        round: Some(round),
                        tool_call_id: Some(call_id),
                        subagent: Some(SubagentProgressDetail {
                            child_iteration: Some(iteration),
                            agent_id: Some(agent_id),
                            task_id: Some(task_id),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
                AgentProgress::SubagentToolCallCompleted {
                    agent_id,
                    task_id,
                    call_id,
                    tool_name,
                    success,
                    output_chars,
                    elapsed_ms,
                    iteration,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_tool_result".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: Some(tool_name),
                        skill_id: Some(task_id.clone()),
                        success: Some(success),
                        round: Some(round),
                        tool_call_id: Some(call_id),
                        output: Some(
                            json!({"output_chars": output_chars, "elapsed_ms": elapsed_ms})
                                .to_string(),
                        ),
                        subagent: Some(SubagentProgressDetail {
                            child_iteration: Some(iteration),
                            agent_id: Some(agent_id),
                            task_id: Some(task_id),
                            elapsed_ms: Some(elapsed_ms),
                            output_chars: Some(output_chars as u64),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
                AgentProgress::SubagentTextDelta {
                    agent_id,
                    task_id,
                    delta,
                    iteration,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_text_delta".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        round: Some(round),
                        delta: Some(delta),
                        delta_kind: Some("text".to_string()),
                        skill_id: Some(task_id.clone()),
                        subagent: Some(SubagentProgressDetail {
                            child_iteration: Some(iteration),
                            agent_id: Some(agent_id),
                            task_id: Some(task_id),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
                AgentProgress::SubagentThinkingDelta {
                    agent_id,
                    task_id,
                    delta,
                    iteration,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_thinking_delta".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        round: Some(round),
                        delta: Some(delta),
                        delta_kind: Some("thinking".to_string()),
                        skill_id: Some(task_id.clone()),
                        subagent: Some(SubagentProgressDetail {
                            child_iteration: Some(iteration),
                            agent_id: Some(agent_id),
                            task_id: Some(task_id),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
                AgentProgress::TaskBoardUpdated { board } => {
                    log::debug!(
                        "[web_channel][bridge] task_board_updated client_id={} thread_id={} request_id={} cards={}",
                        client_id,
                        thread_id,
                        request_id,
                        board.cards.len()
                    );
                    publish_web_channel_event(WebChannelEvent {
                        event: "task_board_updated".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        task_board: Some(serde_json::to_value(board).unwrap_or_else(
                            |_| serde_json::json!({ "threadId": thread_id, "cards": [] }),
                        )),
                        ..Default::default()
                    });
                }
                AgentProgress::TextDelta { delta, iteration } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "text_delta".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        round: Some(iteration),
                        delta: Some(delta),
                        delta_kind: Some("text".to_string()),
                        ..Default::default()
                    });
                }
                AgentProgress::ThinkingDelta { delta, iteration } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "thinking_delta".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        round: Some(iteration),
                        delta: Some(delta),
                        delta_kind: Some("thinking".to_string()),
                        ..Default::default()
                    });
                }
                AgentProgress::ToolCallArgsDelta {
                    call_id,
                    tool_name,
                    delta,
                    iteration,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "tool_args_delta".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: if tool_name.is_empty() {
                            None
                        } else {
                            Some(tool_name)
                        },
                        skill_id: Some("web_channel".to_string()),
                        round: Some(iteration),
                        delta: Some(delta),
                        delta_kind: Some("tool_args".to_string()),
                        tool_call_id: Some(call_id),
                        ..Default::default()
                    });
                }
                AgentProgress::TurnCompleted { iterations } => {
                    log::debug!(
                        "[web_channel] turn completed after {iterations} iteration(s) \
                         client_id={client_id} thread_id={thread_id} request_id={request_id}"
                    );
                }
                AgentProgress::TurnCostUpdated {
                    model,
                    iteration,
                    input_tokens,
                    output_tokens,
                    cached_input_tokens,
                    total_usd,
                } => {
                    // Cost telemetry — not surfaced to the UI yet, but
                    // logged at debug for now and ready for a future
                    // socket payload.
                    log::debug!(
                        "[web_channel] turn cost update model={model} iter={iteration} \
                         in={input_tokens} out={output_tokens} cached_in={cached_input_tokens} \
                         total_usd={total_usd:.4} client_id={client_id} thread_id={thread_id}"
                    );
                }
            }
        }
        turn_state.finish();
        log::debug!(
            "[web_channel][bridge] exit client_id={} thread_id={} request_id={} round={} events_seen={}",
            client_id,
            thread_id,
            request_id,
            round,
            events_seen,
        );
    });
}

fn normalize_model_override(model_override: Option<String>) -> Option<String> {
    model_override
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
}

fn provider_role_for_model_override(model_override: Option<&str>) -> &'static str {
    match model_override.map(str::trim) {
        Some("hint:agentic") | Some("agentic-v1") => "agentic",
        Some("hint:coding") | Some("coding-v1") => "coding",
        Some("hint:summarization") | Some("summarization-v1") => "summarization",
        Some("hint:reasoning") => "reasoning",
        _ => "chat",
    }
}

fn build_session_agent(
    config: &Config,
    client_id: &str,
    thread_id: &str,
    target_agent_id: &str,
    profile: &AgentProfile,
    model_override: Option<String>,
    temperature: Option<f64>,
    locale: Option<&str>,
) -> Result<Agent, String> {
    let mut effective = config.clone();
    if let Some(model) = model_override {
        effective.default_model = Some(model);
    }
    let provider_role = provider_role_for_model_override(effective.default_model.as_deref());
    if let Some(temp) = temperature {
        effective.default_temperature = temp;
    }

    // All chat turns route directly to the orchestrator agent (or to the
    // profile-specific agent for non-default profiles). The welcome agent
    // has been removed; onboarding UI is handled by the Joyride walkthrough
    // in the frontend.
    log::info!(
        "[web-channel] routing chat turn to '{}' via profile '{}' provider_role='{}' (client_id={}, thread_id={})",
        target_agent_id,
        profile.id,
        provider_role,
        client_id,
        thread_id
    );

    // (#623) If this thread was spawned from a subconscious reflection,
    // load the pre-resolved `source_chunks` snapshot and route through
    // the chunks-aware constructor so the orchestrator's system prompt
    // carries the same memory context the reflection-LLM cited. For
    // regular threads this is a no-op (chunks=None, normal path).
    let reflection_chunks = load_reflection_chunks_for_thread(&effective.workspace_dir, thread_id);

    if let Some(chunks) = reflection_chunks
        .as_ref()
        .filter(|chunks| !chunks.is_empty())
    {
        log::info!(
            "[web-channel] thread={} spawned from reflection — injecting {} memory chunks into system prompt",
            thread_id,
            chunks.len()
        );
    }

    // Compose the locale-directive (e.g. "Respond in Arabic") with the
    // profile's own suffix so the agent always reads the user's
    // preferred reply language alongside any profile-level rules. The
    // directive is emitted only for non-English locales — English
    // matches the agent's default, so injecting it would just be noise
    // for the LLM and a regression risk for cached/seeded transcripts.
    let locale_directive = locale.and_then(locale_reply_directive);
    let composed_suffix = compose_system_prompt_suffix(
        locale_directive.as_deref(),
        profile.system_prompt_suffix.as_deref(),
    );
    if let Some(s) = locale_directive.as_deref() {
        log::info!(
            "[web-channel] injecting locale directive client={} thread={} locale={} directive={:?}",
            client_id,
            thread_id,
            locale.unwrap_or(""),
            s
        );
    }

    let agent_result = Agent::from_config_for_agent_with_profile(
        &effective,
        target_agent_id,
        reflection_chunks,
        composed_suffix,
    );

    agent_result
        .map(|mut agent| {
            if let Some(allowed_tools) = profile
                .allowed_tools
                .as_ref()
                .filter(|tools| !tools.is_empty())
            {
                agent.set_visible_tool_names(
                    allowed_tools
                        .iter()
                        .map(|tool| tool.trim().to_string())
                        .filter(|tool| !tool.is_empty())
                        .collect::<HashSet<_>>(),
                );
            }
            agent.set_event_context(event_session_id_for(client_id, thread_id), "web_channel");
            // Scope session transcripts per thread so each conversation
            // gets its own transcript file instead of sharing one by
            // agent type. Without this, new threads load the latest
            // transcript for the agent name and inherit prior messages.
            let short_thread = if thread_id.len() > 12 {
                &thread_id[..12]
            } else {
                thread_id
            };
            agent.set_agent_definition_name(format!("{target_agent_id}_{short_thread}"));
            agent
        })
        .map_err(|e| e.to_string())
}

/// Look up reflection-spawned-thread metadata for a chat thread (#623).
///
/// Reads the thread's first message; if it was seeded by `reflections_act`
/// — `extra_metadata.origin == "subconscious_reflection"` with a
/// `reflection_id` — fetches the reflection row and returns its
/// pre-resolved `source_chunks` snapshot. Returns `None` for ordinary
/// chat threads (no reflection origin) and on any error so a missing
/// reflection never breaks the chat path.
fn load_reflection_chunks_for_thread(
    workspace_dir: &std::path::Path,
    thread_id: &str,
) -> Option<Vec<crate::openhuman::subconscious::SourceChunk>> {
    let messages = crate::openhuman::memory_conversations::get_messages(
        workspace_dir.to_path_buf(),
        thread_id,
    )
    .ok()?;
    let first = messages.first()?;
    let origin = first
        .extra_metadata
        .get("origin")
        .and_then(|v| v.as_str())?;
    if origin != "subconscious_reflection" {
        return None;
    }
    let reflection_id = first
        .extra_metadata
        .get("reflection_id")
        .and_then(|v| v.as_str())?
        .to_string();
    let reflection =
        crate::openhuman::subconscious::store::with_connection(workspace_dir, |conn| {
            crate::openhuman::subconscious::reflection_store::get_reflection(conn, &reflection_id)
        })
        .ok()
        .flatten()?;
    Some(reflection.source_chunks)
}

#[derive(Debug, Deserialize)]
struct WebChatParams {
    client_id: String,
    thread_id: String,
    message: String,
    model_override: Option<String>,
    temperature: Option<f64>,
    profile_id: Option<String>,
    /// BCP-47 locale of the frontend UI (e.g. `ar`, `zh-CN`). When set
    /// and not English, the system prompt is augmented to ask the
    /// agent to reply in that language. `None` keeps the agent's
    /// default language (English) so existing integrations don't
    /// silently change behaviour.
    locale: Option<String>,
    /// Queue mode for concurrent messages: `interrupt` (default), `steer`,
    /// `followup`, or `collect`.
    queue_mode: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WebQueueParams {
    thread_id: String,
}

#[derive(Debug, Deserialize)]
struct WebCancelParams {
    client_id: String,
    thread_id: String,
}

pub async fn channel_web_chat(
    client_id: &str,
    thread_id: &str,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    profile_id: Option<String>,
    locale: Option<String>,
    queue_mode: Option<String>,
) -> Result<RpcOutcome<Value>, String> {
    let result = start_chat(
        client_id,
        thread_id,
        message,
        model_override,
        temperature,
        profile_id,
        locale,
        queue_mode,
    )
    .await?;

    // start_chat returns either a plain request_id string or a JSON string
    // (for queued messages). Try to parse as JSON first.
    if let Ok(parsed) = serde_json::from_str::<Value>(&result) {
        return Ok(RpcOutcome::single_log(parsed, "web channel message queued"));
    }

    Ok(RpcOutcome::single_log(
        json!({
            "accepted": true,
            "client_id": client_id.trim(),
            "thread_id": thread_id.trim(),
            "request_id": result,
        }),
        "web channel request accepted",
    ))
}

pub async fn channel_web_queue_status(thread_id: &str) -> Result<RpcOutcome<Value>, String> {
    let map_key = key_for(thread_id);
    let in_flight = IN_FLIGHT.lock().await;
    if let Some(entry) = in_flight.get(&map_key) {
        let status = entry.run_queue.status().await;
        Ok(RpcOutcome::single_log(
            json!({
                "thread_id": thread_id.trim(),
                "active": true,
                "request_id": entry.request_id,
                "steers": status.steers,
                "followups": status.followups,
                "collects": status.collects,
                "total": status.total,
            }),
            "queue status retrieved",
        ))
    } else {
        Ok(RpcOutcome::single_log(
            json!({
                "thread_id": thread_id.trim(),
                "active": false,
                "steers": 0,
                "followups": 0,
                "collects": 0,
                "total": 0,
            }),
            "no active turn for thread",
        ))
    }
}

pub async fn channel_web_queue_clear(thread_id: &str) -> Result<RpcOutcome<Value>, String> {
    let map_key = key_for(thread_id);
    let in_flight = IN_FLIGHT.lock().await;
    if let Some(entry) = in_flight.get(&map_key) {
        let dropped = entry.run_queue.clear().await;
        log::info!(
            "[web-channel] cleared queue thread_id={} dropped={}",
            thread_id,
            dropped
        );
        Ok(RpcOutcome::single_log(
            json!({
                "thread_id": thread_id.trim(),
                "cleared": true,
                "dropped": dropped,
            }),
            "queue cleared",
        ))
    } else {
        Ok(RpcOutcome::single_log(
            json!({
                "thread_id": thread_id.trim(),
                "cleared": false,
                "dropped": 0,
            }),
            "no active turn for thread",
        ))
    }
}

pub async fn channel_web_cancel(
    client_id: &str,
    thread_id: &str,
) -> Result<RpcOutcome<Value>, String> {
    let cancelled_request_id = cancel_chat(client_id, thread_id).await?;

    Ok(RpcOutcome::single_log(
        json!({
            "cancelled": cancelled_request_id.is_some(),
            "client_id": client_id.trim(),
            "thread_id": thread_id.trim(),
            "request_id": cancelled_request_id,
        }),
        "web channel cancellation processed",
    ))
}

pub fn all_web_channel_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("chat"),
        schemas("cancel"),
        schemas("queue_status"),
        schemas("queue_clear"),
    ]
}

pub fn all_web_channel_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("chat"),
            handler: handle_chat,
        },
        RegisteredController {
            schema: schemas("cancel"),
            handler: handle_cancel,
        },
        RegisteredController {
            schema: schemas("queue_status"),
            handler: handle_queue_status,
        },
        RegisteredController {
            schema: schemas("queue_clear"),
            handler: handle_queue_clear,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "chat" => ControllerSchema {
            namespace: "channel",
            function: "web_chat",
            description: "Send a web channel message through the agent loop.",
            inputs: vec![
                required_string("client_id", "Client stream identifier."),
                required_string("thread_id", "Thread identifier."),
                required_string("message", "User message."),
                optional_string("model_override", "Optional model override."),
                optional_f64("temperature", "Optional temperature override."),
                optional_string("profile_id", "Optional agent profile id."),
                optional_string(
                    "locale",
                    "Optional BCP-47 UI locale (e.g. 'ar', 'zh-CN'). Drives the \"reply in this language\" system-prompt directive.",
                ),
                optional_string(
                    "queue_mode",
                    "Queue mode: 'interrupt' (default), 'steer', 'followup', or 'collect'.",
                ),
            ],
            outputs: vec![json_output("ack", "Acceptance payload.")],
        },
        "cancel" => ControllerSchema {
            namespace: "channel",
            function: "web_cancel",
            description: "Cancel in-flight web channel request for a thread.",
            inputs: vec![
                required_string("client_id", "Client stream identifier."),
                required_string("thread_id", "Thread identifier."),
            ],
            outputs: vec![json_output("ack", "Cancellation payload.")],
        },
        "queue_status" => ControllerSchema {
            namespace: "channel",
            function: "web_queue_status",
            description: "Get the run queue status for a thread.",
            inputs: vec![required_string("thread_id", "Thread identifier.")],
            outputs: vec![json_output("status", "Queue status payload.")],
        },
        "queue_clear" => ControllerSchema {
            namespace: "channel",
            function: "web_queue_clear",
            description: "Clear the run queue for a thread.",
            inputs: vec![required_string("thread_id", "Thread identifier.")],
            outputs: vec![json_output("result", "Queue clear result.")],
        },
        _ => ControllerSchema {
            namespace: "channel",
            function: "unknown",
            description: "Unknown web channel controller function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

fn handle_chat(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<WebChatParams>(params)?;
        to_json(
            channel_web_chat(
                &p.client_id,
                &p.thread_id,
                &p.message,
                p.model_override,
                p.temperature,
                p.profile_id,
                p.locale,
                p.queue_mode,
            )
            .await?,
        )
    })
}

fn handle_queue_status(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<WebQueueParams>(params)?;
        to_json(channel_web_queue_status(&p.thread_id).await?)
    })
}

fn handle_queue_clear(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<WebQueueParams>(params)?;
        to_json(channel_web_queue_clear(&p.thread_id).await?)
    })
}

/// Map a frontend BCP-47 locale tag to a system-prompt directive
/// instructing the agent to reply in that language. Returns `None`
/// for English (the agent's default — adding "Respond in English"
/// is a no-op for the LLM but risks invalidating cached prefixes)
/// and for unknown tags so the agent falls through to its default
/// behaviour instead of seeing a half-built directive.
pub(crate) fn locale_reply_directive(locale: &str) -> Option<String> {
    let language = match locale.trim() {
        // Keep this table in lockstep with `Locale` in
        // `app/src/lib/i18n/types.ts` — every locale the frontend can
        // ship should resolve to a language name here.
        "ar" => "Arabic",
        "bn" => "Bengali",
        "es" => "Spanish",
        "fr" => "French",
        "hi" => "Hindi",
        "id" => "Indonesian",
        "it" => "Italian",
        "pt" => "Portuguese",
        "ru" => "Russian",
        "zh-CN" | "zh" => "Simplified Chinese",
        // English (and any unrecognised tag) → no directive.
        _ => return None,
    };
    Some(format!(
        "User language: the user's interface is set to {language}. \
         Respond in {language} unless the user explicitly asks for a different language. \
         Keep proper nouns, code, and command names untranslated."
    ))
}

/// Stitch the locale directive (if any) onto the profile's own
/// system-prompt suffix. The directive comes first so it shows up
/// near the top of the appended block — easier for the LLM to honour
/// than language guidance buried after profile-specific rules.
pub(crate) fn compose_system_prompt_suffix(
    locale_directive: Option<&str>,
    profile_suffix: Option<&str>,
) -> Option<String> {
    match (locale_directive, profile_suffix) {
        (None, None) => None,
        (Some(d), None) => Some(d.to_string()),
        (None, Some(p)) => Some(p.to_string()),
        (Some(d), Some(p)) => Some(format!("{d}\n\n{p}")),
    }
}

fn handle_cancel(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<WebCancelParams>(params)?;
        to_json(channel_web_cancel(&p.client_id, &p.thread_id).await?)
    })
}

fn deserialize_params<T: serde::de::DeserializeOwned>(
    params: Map<String, Value>,
) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

fn optional_f64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
        comment,
        required: false,
    }
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "web_tests.rs"]
mod tests;

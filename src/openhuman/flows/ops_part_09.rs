
/// Cancels a flow run (issue G4), settling it to a terminal `"cancelled"`
/// status and dropping its durable checkpoint so the aborted thread can never
/// be resumed.
///
/// Two cases, distinguished by [`run_registry::cancel`]:
/// - **In-flight** (a `flows_run` / `flows_resume` currently executing its run
///   future): the token is signalled and that run's own cancellation arm writes
///   the terminal row + drops the checkpoint as it unwinds — we don't write the
///   row here, to avoid two writers racing the same `flow_runs` row.
/// - **Parked / stale** (a `pending_approval` run awaiting a human decision, or
///   a `running` row whose task is gone): no live task exists to unwind, so
///   this settles the row terminally itself and drops the checkpoint.
///
/// A run that is already terminal (`completed` / `completed_with_warnings` /
/// `failed` / `cancelled` / `interrupted`) is a clear error, not a silent
/// no-op — otherwise a settled warning run could be overwritten as
/// `"cancelled"`, corrupting the run-honesty status it already recorded, and an
/// already-`interrupted` run (reconciled by the drop-guard / boot sweep, bug
/// B42) could be clobbered back to `"cancelled"`.
pub async fn flows_cancel_run(config: &Config, run_id: &str) -> Result<RpcOutcome<Value>, String> {
    let run = store::get_flow_run(config, run_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("flow run '{run_id}' not found"))?;

    if matches!(
        run.status.as_str(),
        "completed" | "completed_with_warnings" | "failed" | "cancelled" | "interrupted"
    ) {
        return Err(format!(
            "flow run '{run_id}' is already terminal (status: {}) — nothing to cancel",
            run.status
        ));
    }

    let signalled = run_registry::cancel(run_id);
    tracing::info!(
        target: "flows",
        run_id,
        flow_id = %run.flow_id,
        signalled,
        prior_status = %run.status,
        "[flows] flows_cancel_run: cancelling run"
    );

    if signalled {
        // The in-flight run's cancellation arm owns the terminal write + the
        // checkpoint drop; we've signalled it and return. Its settle is
        // eventual (the run future unwinds), so report "requested".
        return Ok(RpcOutcome::single_log(
            json!({ "run_id": run_id, "cancelled": true, "was_in_flight": true }),
            format!("flow run {run_id} cancellation requested"),
        ));
    }

    // Not in flight: settle the row terminally and drop the checkpoint here.
    //
    // ORDER MATTERS (R-M2). The status read above and `run_registry::cancel`
    // are two separate observations, and a live run can settle in the window
    // between them: it writes its own terminal row and deregisters, so
    // `cancel` returns `false` and we arrive here believing the run is merely
    // parked/stale. Writing `cancelled` unconditionally would then relabel a
    // fully-completed run — whose real side effects already fired — and drop a
    // checkpoint that is no longer ours to drop. So attempt the guarded row
    // write FIRST and treat it as the authority: it only matches a still-live
    // row, so `false` means the run settled underneath us. Only once it has
    // won do we record the flow summary and drop the checkpoint.
    let observed = current_persisted_steps(config, run_id);
    let settled_by_us = finish_flow_run_row(
        config,
        run_id,
        &run.flow_id,
        "cancelled",
        &observed,
        &[],
        Some("run cancelled"),
        None,
    );
    if !settled_by_us {
        tracing::info!(
            target: "flows",
            run_id,
            flow_id = %run.flow_id,
            prior_status = %run.status,
            "[flows] flows_cancel_run: run settled concurrently — leaving its terminal status intact"
        );
        return Err(format!(
            "flow run '{run_id}' settled before it could be cancelled — its recorded outcome was \
             left untouched"
        ));
    }
    if let Err(e) = store::record_run(config, &run.flow_id, "cancelled") {
        tracing::warn!(target: "flows", run_id, flow_id = %run.flow_id, error = %e, "[flows] flows_cancel_run: failed to record cancelled status on flow summary");
    }
    drop_checkpoint(config, run_id).await;

    Ok(RpcOutcome::single_log(
        json!({ "run_id": run_id, "cancelled": true, "was_in_flight": false }),
        format!("flow run {run_id} cancelled"),
    ))
}

/// Best-effort drop of a run's durable tinyagents checkpoint thread, so a
/// cancelled (or expired) run can never be resumed from its persisted interrupt
/// boundary. Logged, never fatal — the `flow_runs` row's terminal status is the
/// authoritative "not resumable" signal (the `flows_resume` guard already
/// rejects any non-`pending_approval` status); dropping the checkpoint is
/// belt-and-suspenders that also reclaims the storage.
async fn drop_checkpoint(config: &Config, thread_id: &str) {
    match crate::openhuman::flows::tinyflows::open_flow_checkpointer(config) {
        Ok(checkpointer) => match checkpointer.delete_thread(thread_id).await {
            Ok(()) => {
                tracing::debug!(target: "flows", thread_id, "[flows] dropped durable checkpoint for cancelled/expired run")
            }
            Err(e) => {
                tracing::warn!(target: "flows", thread_id, error = %e, "[flows] failed to drop durable checkpoint")
            }
        },
        Err(e) => {
            tracing::warn!(target: "flows", thread_id, error = %e, "[flows] could not open checkpointer to drop checkpoint");
        }
    }
}

/// Builds the `TrustedAutomation { Workflow }` origin scoped around every
/// `flows_run` / `flows_resume` invocation. See `flows_run`'s doc for why
/// this applies uniformly regardless of caller.
fn workflow_origin(flow_id: &str, require_approval: bool) -> AgentTurnOrigin {
    AgentTurnOrigin::TrustedAutomation {
        job_id: flow_id.to_string(),
        source: TrustedAutomationSource::Workflow { require_approval },
    }
}

/// RFC3339 instant at which THIS process first entered the flow-run lifecycle —
/// the floor the boot orphan sweep (bug B42) uses to bound its candidate set.
///
/// Initialized on first touch by whichever comes first: [`start_flow_run_row`]
/// (which forces it *before* stamping the row it is about to insert) or
/// [`sweep_orphaned_running_runs_on_boot`]. Either ordering yields the same
/// invariant — **every `flow_runs` row this process inserts has
/// `started_at >= *PROCESS_RUN_FLOOR`** — so a sweep restricted to
/// `started_at < *PROCESS_RUN_FLOOR` provably only ever sees rows left behind by
/// a *prior* process.
///
/// The floor makes that guarantee structural rather than a consequence of
/// registration ordering. `run_registry::is_in_flight` alone once left a window
/// — the entry points used to insert the `running` row before `run_flow_body`
/// registered, so a live run was briefly `running`-but-not-in-flight, and
/// sweeping it there would `drop_checkpoint` it mid-run (unrecoverable, unlike
/// the status, which the live run's own terminal write would fix). Registration
/// has since moved ahead of the insert, closing that window at the source too;
/// the floor stays because it holds regardless of what future callers do with
/// that ordering.
static PROCESS_RUN_FLOOR: LazyLock<String> = LazyLock::new(|| Utc::now().to_rfc3339());

/// Best-effort insert of the initial `"running"` `flow_runs` row. Logged,
/// never fails the run — run-history persistence is an observability aid,
/// not a correctness requirement of the run itself.
fn start_flow_run_row(config: &Config, thread_id: &str, flow_id: &str) {
    // Anchor the boot-sweep floor BEFORE stamping this row, so this row's
    // `started_at` can never precede it. See [`PROCESS_RUN_FLOOR`].
    LazyLock::force(&PROCESS_RUN_FLOOR);
    let started_at = Utc::now().to_rfc3339();
    if let Err(e) = store::insert_flow_run(config, thread_id, flow_id, thread_id, &started_at) {
        tracing::warn!(target: "flows", flow_id, thread_id, error = %e, "[flows] failed to persist flow run start");
    }
}

/// Best-effort finalization of a `flow_runs` row. Logged, never fails the
/// run (see [`start_flow_run_row`]).
///
/// `graph_hash` (T-M1) should be `Some(hash)` only on the write that parks the
/// row (`status == "pending_approval"`) — every other caller passes `None`,
/// which clears any stale pin now that the row is leaving (or never entered)
/// `pending_approval`. See [`compute_graph_hash`] and `store::finish_flow_run`.
fn finish_flow_run_row(
    config: &Config,
    thread_id: &str,
    flow_id: &str,
    status: &str,
    steps: &[FlowRunStep],
    pending_approvals: &[String],
    error: Option<&str>,
    graph_hash: Option<&str>,
) -> bool {
    let finished_at = Utc::now().to_rfc3339();
    match store::finish_flow_run(
        config,
        thread_id,
        status,
        &finished_at,
        steps,
        pending_approvals,
        error,
        graph_hash,
    ) {
        Err(e) => {
            tracing::warn!(target: "flows", thread_id, status, error = %e, "[flows] failed to persist flow run finish");
            return false;
        }
        // The guarded UPDATE (R-M2) matched nothing: the row had already
        // settled to a terminal status before this write. Whoever settled it
        // first also published `FlowRunFinished`, so publishing again here
        // would emit a second terminal event for one run. Report the no-op
        // instead of pretending the write landed.
        Ok(false) => {
            tracing::warn!(
                target: "flows",
                flow_id,
                thread_id,
                attempted_status = status,
                "[flows] finish_flow_run_row: row already terminal — refusing to overwrite a settled run"
            );
            return false;
        }
        Ok(true) => {}
    }

    // `status` can be `"pending_approval"` here (see `finalize_terminal_status`)
    // when the run merely paused at a gate — that isn't a finish. `flows_resume`
    // later settles under the SAME `thread_id`/`run_id`, and `useFlowRunFinished`
    // de-dupes delivered events by `${flow_id}:${run_id}` (needed because the
    // socket bridge re-emits this event under two aliases and must collapse
    // them into one `onFinish` call). Publishing here for a pause would poison
    // that dedup cache, so the real completion event after resume would be
    // dropped as an "alias replay" and the run could stay stale in the runs
    // list until the 30s poll backstop (Codex review, PR #5115). Gate the
    // publish to actual terminal statuses; the row itself is still written
    // above so poll-based fallbacks (list/get RPCs) see the paused state
    // either way.
    if status == "pending_approval" {
        tracing::debug!(
            target: "flows",
            flow_id,
            thread_id,
            status,
            "[flows] finish_flow_run_row: run paused for approval — not a finish, skipping FlowRunFinished"
        );
        return true;
    }

    tracing::debug!(
        target: "flows",
        flow_id,
        thread_id,
        status,
        "[flows] finish_flow_run_row: publishing FlowRunFinished"
    );
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::FlowRunFinished {
        flow_id: flow_id.to_string(),
        run_id: thread_id.to_string(),
        status: status.to_string(),
    });
    true
}

/// Computes a stable content hash of the flow configuration a run was approved
/// against — the T-M1 stale-approval guard (see `flows_resume`'s doc).
/// Persisted on a run row the moment it parks at `pending_approval`, and
/// recompared against the **current** flow before a resume is allowed to
/// execute, so a rewrite between park and resume is detected instead of
/// silently firing the new configuration under the old approval.
///
/// Covers the graph **and `require_approval`**. The flag is not cosmetic: it
/// feeds `workflow_origin(...)`, which becomes the `AgentTurnOrigin` for the
/// whole resumed execution, and `TrustedAutomationSource::Workflow {
/// require_approval: false }` **auto-allows every `external_effect` tool call**
/// where `true` parks each one for its own human decision. It is also settable
/// independently of the graph — `flows_update(.., graph_json: None,
/// require_approval: Some(false), ..)` leaves `.graph` byte-identical. Hashing
/// the graph alone would therefore leave the exact hole this guard exists to
/// close: park at a gate, user approves, the flag is flipped to `false` with the
/// graph untouched (pin still matches), and on resume every downstream
/// outbound node that would have parked now fires unattended.
///
/// Hashes a *canonicalized* JSON serialization — `serde_json::Value`'s object
/// map preserves insertion order in this crate (the `preserve_order` feature
/// is enabled transitively via other dependencies), so the same logical graph
/// serialized through two different code paths is not guaranteed to emit its
/// object keys in the same order. [`canonicalize_json`] recursively sorts
/// every object's keys before hashing so the hash depends only on graph
/// content, never on incidental key order. Returns `None` (never panics) if
/// the graph somehow fails to serialize.
///
/// **`None` means different things on the two sides, and the resume side fails
/// CLOSED.** At park time `None` simply stores no pin, so that run later takes
/// the legacy "unknown — allow, with a warning" path. At resume time the
/// comparison is `Some(expected) != None`, which is *true*, so a hash failure
/// is treated as a mismatch: the run is refused, settled terminally, and its
/// checkpoint dropped. That is the safer direction — a run whose current graph
/// cannot be hashed is a run whose approval cannot be verified — but it is the
/// opposite of fail-open, so do not read this as a guarantee that a serialize
/// failure leaves a resumable run resumable.
fn compute_graph_hash(graph: &WorkflowGraph, require_approval: bool) -> Option<String> {
    let raw = match serde_json::to_value(graph) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                target: "flows",
                error = %e,
                "[flows] compute_graph_hash: failed to serialize graph to JSON — proceeding without a graph pin"
            );
            return None;
        }
    };
    let raw = serde_json::json!({ "graph": raw, "require_approval": require_approval });
    let canonical = canonicalize_json(&raw);
    let serialized = match serde_json::to_string(&canonical) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                target: "flows",
                error = %e,
                "[flows] compute_graph_hash: failed to serialize canonicalized graph — proceeding without a graph pin"
            );
            return None;
        }
    };
    let digest = Sha256::digest(serialized.as_bytes());
    Some(hex::encode(digest))
}

/// Recursively rewrites every JSON object's keys into sorted order, leaving
/// arrays (whose element order is semantically meaningful) and scalars
/// unchanged. See [`compute_graph_hash`] for why this is needed before
/// hashing rather than trusting `serde_json`'s default map order.
fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut sorted = serde_json::Map::new();
            for key in keys {
                sorted.insert(key.clone(), canonicalize_json(&map[key]));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json).collect()),
        other => other.clone(),
    }
}

/// Reconstructs a lean per-node step list from a settled run's
/// `output["nodes"]` map.
///
/// As of issue G2 (live run observation) this is no longer the primary source
/// of run steps — `flows::observability::FlowRunObserver` persists each step
/// live as it finishes (with real `status`/`duration_ms`). This reconstruction
/// is now only a **fallback**, used by [`settle_steps`] to fill in any node the
/// observer didn't emit an `on_step_finish` for (notably the trigger node),
/// and as the whole-run source when the observer saw nothing at all.
fn reconstruct_steps(output: &Value) -> Vec<FlowRunStep> {
    let Some(nodes) = output.get("nodes").and_then(Value::as_object) else {
        return Vec::new();
    };
    nodes
        .iter()
        .map(|(node_id, slot)| FlowRunStep {
            node_id: node_id.clone(),
            output: slot.get("items").cloned().unwrap_or(Value::Null),
            port: slot.get("port").and_then(Value::as_str).map(str::to_string),
            // Reconstructed post-hoc: no live status/timing (see FlowRunStep).
            status: None,
            duration_ms: None,
            diagnostics: Vec::new(),
        })
        .collect()
}

/// Reads back whatever steps the live [`FlowRunObserver`] has already persisted
/// onto the run's row. Best-effort: a read failure yields an empty list (the
/// caller still writes a terminal row), never propagating an error into the
/// run's settle path.
///
/// [`FlowRunObserver`]: crate::openhuman::flows::tinyflows::observability::FlowRunObserver
fn current_persisted_steps(config: &Config, run_id: &str) -> Vec<FlowRunStep> {
    store::get_flow_run(config, run_id)
        .ok()
        .flatten()
        .map(|run| run.steps)
        .unwrap_or_default()
}

/// Assembles the final step list to persist at settle: the live steps the
/// observer already recorded (carrying real `status`/`duration_ms`), plus any
/// node present in the post-hoc [`reconstruct_steps`] projection that the
/// observer never emitted a step for — the trigger node, or (defensively) an
/// observer that missed a step. If the observer recorded nothing at all
/// (e.g. a run that paused immediately at a gate before any node finished),
/// falls back wholesale to the reconstruction.
fn settle_steps(config: &Config, run_id: &str, output: &Value) -> Vec<FlowRunStep> {
    let reconstructed = reconstruct_steps(output);
    let persisted = current_persisted_steps(config, run_id);
    if persisted.is_empty() {
        tracing::debug!(
            target: "flows",
            run_id,
            reconstructed = reconstructed.len(),
            "[flows] settle_steps: no live-observed steps — using post-hoc reconstruction"
        );
        return reconstructed;
    }
    let mut merged = persisted;
    let mut filled = 0usize;
    for step in reconstructed {
        if !merged.iter().any(|s| s.node_id == step.node_id) {
            merged.push(step);
            filled += 1;
        }
    }
    tracing::debug!(
        target: "flows",
        run_id,
        step_count = merged.len(),
        filled_from_reconstruction = filled,
        "[flows] settle_steps: merged live-observed steps with post-hoc reconstruction"
    );
    merged
}

/// Degrades a would-be `"completed"` status: `"failed"` if any settled step
/// errored, `"completed_with_warnings"` if any carries null-resolution
/// diagnostics, else `"completed"`.
///
/// Called only once the run has no `pending_approvals` left — precedence
/// against that case is handled by the caller (`pending_approval` always
/// wins over any of these).
fn degrade_completed_status(steps: &[FlowRunStep]) -> &'static str {
    if steps.iter().any(|s| s.status.as_deref() == Some("error")) {
        return "failed";
    }
    if steps.iter().any(|s| !s.diagnostics.is_empty()) {
        "completed_with_warnings"
    } else {
        "completed"
    }
}

/// Names the node(s) whose step settled with `status == "error"` — the
/// engine's `ExecutionStep` carries no error message of its own for a step
/// that failed under an `on_error: "continue"`/`"route"` policy (it only
/// fails the *run* future, and so gets an actual error string, when the
/// policy is `"stop"`), so this is the best available detail for
/// [`FlowRun::error`] when [`degrade_completed_status`] degrades to
/// `"failed"` without an outer run-future `Err`.
fn failed_step_error_summary(steps: &[FlowRunStep]) -> Option<String> {
    let failed_nodes: Vec<&str> = steps
        .iter()
        .filter(|s| s.status.as_deref() == Some("error"))
        .map(|s| s.node_id.as_str())
        .collect();
    if failed_nodes.is_empty() {
        None
    } else {
        Some(format!(
            "node(s) failed after retries: {}",
            failed_nodes.join(", ")
        ))
    }
}

/// Computes a settled run's terminal status and, when that status is
/// `"failed"`, an accompanying error message — shared by `flows_run` and
/// `flows_resume` so the two call sites can't drift on the
/// `pending_approval` > `degrade_completed_status` precedence or forget to
/// populate [`FlowRun::error`] (its doc contract: "Error message when
/// `status == \"failed\"`") for a run that degraded via a settled step error
/// rather than an outer run-future `Err`.
fn finalize_terminal_status(
    settled: &[FlowRunStep],
    pending_approvals: &[String],
) -> (&'static str, Option<String>) {
    if !pending_approvals.is_empty() {
        return ("pending_approval", None);
    }
    let status = degrade_completed_status(settled);
    let error = if status == "failed" {
        failed_step_error_summary(settled)
    } else {
        None
    };
    (status, error)
}

/// Milliseconds since the Unix epoch, for `CoreNotificationEvent::timestamp_ms`.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Surfaces a paused run as a `CoreNotification` (category `Agents`) with an
/// "approve" action carrying `flow_id`/`thread_id`/`node_ids`, mirroring the
/// pattern `agent_meetings::calendar`'s auto-summarize "Ask" flow uses
/// (direct `publish_core_notification` call with an action payload, not the
/// generic `DomainEvent -> event_to_notification` bridge — this is a
/// flows-specific card with flow-specific action data, not a translation of
/// an existing broadcast event). No-op when nothing is pending.
fn notify_pending_approval(flow: &Flow, thread_id: &str, pending_approvals: &[String]) {
    if pending_approvals.is_empty() {
        return;
    }

    use crate::openhuman::desktop::notifications::bus::publish_core_notification;
    use crate::openhuman::desktop::notifications::types::{
        CoreNotificationAction, CoreNotificationCategory, CoreNotificationEvent,
    };

    let action_payload = json!({
        "flow_id": flow.id,
        "thread_id": thread_id,
        "node_ids": pending_approvals,
    });

    publish_core_notification(CoreNotificationEvent {
        id: format!("flow-pending-approval:{}:{}", flow.id, thread_id),
        category: CoreNotificationCategory::Agents,
        title: "Workflow needs approval".to_string(),
        body: format!(
            "\"{}\" is waiting on {} approval{} before it can continue.",
            flow.name,
            pending_approvals.len(),
            if pending_approvals.len() == 1 {
                ""
            } else {
                "s"
            }
        ),
        // No dedicated Workflows review route exists yet (B3 ships the UI);
        // leave unset rather than link to a page that can't act on it.
        deep_link: None,
        timestamp_ms: now_ms(),
        actions: Some(vec![CoreNotificationAction {
            action_id: "approve".to_string(),
            label: "Review".to_string(),
            payload: Some(action_payload),
        }]),
        workspace: None,
        workspace_revision: None,
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Flow Scout — workflow discovery + suggestion lifecycle
// ─────────────────────────────────────────────────────────────────────────────

/// Overall safety bound on one `flows_discover` run. The `flow_discovery` agent
/// reasons read-only over the user's data and ends by emitting
/// `suggest_workflows`; its own `max_iterations` caps the loop, but a hung
/// LLM/tool call must never let the RPC block indefinitely.
///
/// Matches [`FLOW_BUILD_TIMEOUT_SECS`] (600s): the session builder applies the
/// `flow_discovery` definition's `effective_max_iterations()` (50, not the
/// global default of 10) to this path (issue #4868), so a worst-case run at
/// ~10s/iteration can take up to ~500s — the old 300s bound could clip a
/// legitimate long discovery run before the iteration cap ever got a chance
/// to (post-merge Codex P2 finding).
const FLOW_DISCOVER_TIMEOUT_SECS: u64 = 600;

/// The canned brief handed to the `flow_discovery` agent. The agent's own
/// archetype prompt teaches the read → correlate → ground → emit loop; this is
/// just the kick-off instruction for the on-demand "Discover" action.
const FLOW_DISCOVER_PROMPT: &str = "Discover the most useful automations you could set up for me. \
     Read what you can about how I work — my goals, recurring conversations, the people and apps I \
     deal with, and the flows I already have — then propose a few concrete, buildable workflows. \
     Ground each in something you actually observed about me, and end by calling suggest_workflows.";

// ─────────────────────────────────────────────────────────────────────────────
// Copilot / scout streaming (Phase B) — bridge a builder/scout turn's live
// AgentProgress onto the web-channel socket, keyed by a chat thread, exactly
// like an interactive chat turn. Blueprint: `agent/task_dispatcher/executor.rs`.
// ─────────────────────────────────────────────────────────────────────────────

/// Where to stream a `flows_build` / `flows_discover` turn. When present, the
/// agent's progress events (`text_delta` / `thinking_delta` / `tool_call` /
/// `tool_result` / terminal `chat_done`) are published as `WebChannelEvent`s
/// tagged with this `thread_id` — the same room the shared chat pane already
/// subscribes to and decodes — so the copilot/scout UI renders streamed text,
/// tool cards, and workflow-proposal cards live instead of spinning for the
/// whole (up to 300s) headless run.
///
/// Broadcast client id is always `"system"` (like cron / task-session runs), so
/// any client viewing the thread receives the events (the frontend keys by
/// `thread_id`). The blocking `{ proposal, assistant_text }` return is
/// unchanged — streaming is purely additive, opt-in per call.
#[derive(Debug, Clone)]
pub struct FlowStreamTarget {
    /// The chat thread the copilot/scout turn streams into.
    pub thread_id: String,
    /// Per-turn correlation id (matches the frontend `request_id`). Generated
    /// when the caller doesn't supply one.
    pub request_id: String,
}

impl FlowStreamTarget {
    /// Build a streaming target from optional RPC params. Streaming is enabled
    /// only when a non-empty `thread_id` is given; a missing/blank `request_id`
    /// is filled with a fresh uuid so the turn is always correlatable. Returns
    /// `None` (headless run, prior behaviour) when no usable `thread_id`.
    pub fn from_params(thread_id: Option<String>, request_id: Option<String>) -> Option<Self> {
        let thread_id = thread_id
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())?;
        let request_id = request_id
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        Some(Self {
            thread_id,
            request_id,
        })
    }
}

/// Attach the web-channel progress bridge to `agent` for a builder/scout turn.
/// Wires an mpsc channel into the agent's progress sink and spawns the bridge
/// task that translates each [`AgentProgress`] into a socket event keyed by the
/// target thread (and mirrors a `TurnStateStore` so the tool timeline replays
/// on reopen). The bridge task lives until the agent drops its progress sender
/// (turn end). `source` is a short trace-attribution label (e.g.
/// `"flows_build"`).
fn attach_flow_progress_bridge(
    agent: &mut crate::openhuman::agent::Agent,
    target: &FlowStreamTarget,
    source: &str,
    config: &Config,
) {
    let (progress_tx, progress_rx) = tokio::sync::mpsc::channel(64);
    agent.set_on_progress(Some(progress_tx));
    tracing::info!(
        target: "flows",
        thread_id = %target.thread_id,
        request_id = %target.request_id,
        source = %source,
        "[flows] progress bridge: attaching (streaming copilot/scout turn)"
    );
    crate::openhuman::web_chat::spawn_progress_bridge(
        progress_rx,
        "system".to_string(),
        target.thread_id.clone(),
        target.request_id.clone(),
        crate::openhuman::threads::turn_state::TurnStateStore::new(config.workspace_dir.clone()),
        crate::openhuman::web_chat::ChatRequestMetadata {
            source: Some(source.to_string()),
            ..Default::default()
        },
        config.clone(),
    );
}

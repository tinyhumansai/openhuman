//! Tools: `run_workflow` + `await_workflow` — let the orchestrator compose
//! workflows by running one as a subagent and (optionally) waiting on its
//! result, the way a function call waits on its callee.
//!
//! `run_workflow` spawns a target workflow as a fresh autonomous background
//! run (its own log, its own iter cap), then **awaits the result inside the
//! tool** for up to `wait_seconds`. The polling happens in the runtime — a
//! tokio sleep loop over the run's log footer — NOT in the LLM: the model
//! issues one tool call and gets back either the finished `status` + `output`
//! or, if the run outlives the wait budget, a `status: "running"` handle it
//! can re-attach to later. That auto-detach is what keeps a long shepherd-
//! style run from freezing the caller forever.
//!
//! `await_workflow` re-attaches to a detached run by `run_id` and waits the
//! same way — so a workflow can kick several children off with
//! `wait_seconds: 0` and then collect them.
//!
//! Composition example: `github-issue-crusher` opens a draft PR, then calls
//! `run_workflow` with `workflow_id: "pr-review-shepherd"` and the PR number.
//! If the shepherd finishes its first pass quickly the crusher gets the
//! result inline; if not, it gets a `run_id` and can move on.
//!
//! Guardrails (see the `guard` module): a per-agent spawn backstop,
//! a concurrency/nesting cap on synchronous awaits, and a re-entrancy lock
//! keyed on workflow-id + inputs so an LLM that loses track can't tip a
//! legitimate A→B→A chain into an unbounded loop. These are deliberately
//! coarse process-global bounds, not per-task-lineage budgets — the detached
//! `tokio::spawn` run path doesn't thread a parent run-id into its children,
//! so true per-lineage depth would need that plumbing first. The coarse
//! bounds still stop the realistic failure modes (fan-out bomb, tight
//! self-loop) without it.

use async_trait::async_trait;
use serde_json::json;

use crate::skills::runtime::{await_run_outcome, spawn_workflow_run_background};
use crate::skills::schemas::resolve_workspace_dir;
use tinytools::{PermissionLevel, Tool, ToolResult};

/// Tool name surfaced to the LLM's function-calling schema.
pub const RUN_WORKFLOW_TOOL_NAME: &str = "run_workflow";
/// Companion tool: re-attach to a detached run and keep waiting.
pub const AWAIT_WORKFLOW_TOOL_NAME: &str = "await_workflow";

/// Default seconds a `run_workflow` / `await_workflow` call waits inline
/// before auto-detaching. Quick workflows return their result directly;
/// slow ones hand back a `run_id`.
const DEFAULT_WAIT_SECONDS: u64 = 90;
/// Hard ceiling on a single inline wait so one tool call can't block a
/// caller indefinitely.
const MAX_WAIT_SECONDS: u64 = 600;

/// Coarse, process-global spawn/await guardrails. See the module doc for why
/// these are global rather than per-lineage.
mod guard {
    use crate::core::runtime::DispatchCtx;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    /// Per-agent lifetime backstop against a runaway spawn loop.
    const TOTAL_SPAWN_BACKSTOP: u64 = 500;
    /// Max workflows being synchronously awaited at once. Because an awaiting
    /// call holds its slot for the whole nested wait, this also bounds the
    /// depth of synchronous workflow→workflow chains.
    const MAX_ACTIVE_AWAITS: u64 = 8;

    /// The spawn counter, await count and re-entrancy keys of one agent
    /// context.
    #[derive(Default)]
    struct GuardState {
        total_spawns: AtomicU64,
        active_awaits: AtomicU64,
        active_keys: Mutex<HashSet<String>>,
    }

    fn state(ctx: &DispatchCtx) -> Arc<GuardState> {
        crate::core::runtime::slot_for::<GuardState>(ctx)
    }

    /// RAII guard held while a call awaits a run. Dropping it frees the
    /// active-await slot and clears the re-entrancy key.
    pub struct AwaitGuard {
        key: String,
        state: Arc<GuardState>,
    }

    impl Drop for AwaitGuard {
        fn drop(&mut self) {
            self.state.active_awaits.fetch_sub(1, Ordering::SeqCst);
            if let Ok(mut keys) = self.state.active_keys.lock() {
                keys.remove(&self.key);
            }
        }
    }

    /// Account a spawn against the agent's backstop. Returns `Err`
    /// once the cap trips. Called by both the awaited and the fire-and-forget
    /// paths so neither can loop forever.
    pub fn account_spawn(ctx: &DispatchCtx) -> Result<(), String> {
        let n = state(ctx).total_spawns.fetch_add(1, Ordering::SeqCst) + 1;
        if n > TOTAL_SPAWN_BACKSTOP {
            return Err(format!(
                "refused — spawn backstop hit ({TOTAL_SPAWN_BACKSTOP} workflow runs \
                 spawned this session). This guards against a runaway spawn loop."
            ));
        }
        Ok(())
    }

    /// Test-only reader for the agent's spawn counter. Used by the
    /// regression test that asserts a rejected spawn (e.g. unknown workflow
    /// id) doesn't consume a backstop slot.
    #[cfg(test)]
    pub fn total_spawns(ctx: &DispatchCtx) -> u64 {
        state(ctx).total_spawns.load(Ordering::SeqCst)
    }

    /// Acquire an await slot + re-entrancy lock for `key` (a workflow-id +
    /// inputs fingerprint, or `await:<run_id>` for re-attach). `Err` if too
    /// many awaits are in flight (nesting/fan-out cap) or the same key is
    /// already being awaited up the stack (re-entrant tight loop).
    pub fn acquire_await(ctx: &DispatchCtx, key: String) -> Result<AwaitGuard, String> {
        let state = state(ctx);
        let mut keys = state
            .active_keys
            .lock()
            .map_err(|_| "internal guard lock poisoned".to_string())?;
        if keys.contains(&key) {
            return Err(
                "refused — this exact workflow + inputs is already being awaited higher up the \
                 call chain (re-entrant loop). Wait for it to finish or vary the inputs."
                    .to_string(),
            );
        }
        if state.active_awaits.load(Ordering::SeqCst) >= MAX_ACTIVE_AWAITS {
            return Err(format!(
                "refused — {MAX_ACTIVE_AWAITS} workflows are already being awaited concurrently \
                 (nesting/fan-out cap). Let some finish, or spawn with `wait_seconds: 0` to \
                 detach instead of awaiting."
            ));
        }
        keys.insert(key.clone());
        state.active_awaits.fetch_add(1, Ordering::SeqCst);
        drop(keys);
        Ok(AwaitGuard { key, state })
    }
}

/// Pull the requested inline wait (seconds) from a tool-call arg map,
/// defaulting + clamping to the supported range.
fn parse_wait_seconds(args: &serde_json::Value) -> u64 {
    args.get("wait_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_WAIT_SECONDS)
        .min(MAX_WAIT_SECONDS)
}

/// Fingerprint a (workflow_id, inputs) pair for the re-entrancy guard.
/// Object keys are sorted recursively so logically identical inputs
/// produce the same key regardless of insertion order.
fn reentrancy_key(workflow_id: &str, inputs: &Option<serde_json::Value>) -> String {
    fn canonicalize(v: &serde_json::Value) -> serde_json::Value {
        match v {
            serde_json::Value::Object(map) => {
                let mut sorted: Vec<_> = map.iter().collect();
                sorted.sort_by_key(|(left, _)| *left);
                serde_json::Value::Object(
                    sorted
                        .into_iter()
                        .map(|(k, v)| (k.clone(), canonicalize(v)))
                        .collect(),
                )
            }
            serde_json::Value::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(canonicalize).collect())
            }
            other => other.clone(),
        }
    }

    let inputs_repr = inputs
        .as_ref()
        .map(|v| canonicalize(v).to_string())
        .unwrap_or_else(|| "null".to_string());
    format!("{workflow_id}\u{1}{inputs_repr}")
}

/// Shape the terminal/"still running" outcome of a wait into a `ToolResult`.
fn outcome_to_result(
    run_id: &str,
    workflow_id: &str,
    log_path: &std::path::Path,
    outcome: Option<crate::skills::run_log::RunOutcome>,
) -> ToolResult {
    match outcome {
        // The harness stopped the run's turn early: it did not finish, so the
        // caller's failure policy and trace must see a failure, not a result.
        Some(o) if o.status == "STOPPED" => {
            tracing::debug!(
                run_id,
                workflow_id,
                "[run_workflow] run was stopped early; returning a failed tool result"
            );
            ToolResult::error(
                json!({
                    "run_id": run_id,
                    "workflow_id": workflow_id,
                    "status": o.status,
                    "error": "The workflow run was stopped before it finished. Do not report it \
                              as done; relay the blocker below or try a different approach.",
                    "output": o.output,
                    "log": log_path.display().to_string(),
                })
                .to_string(),
            )
        }
        Some(o) => ToolResult::success(
            json!({
                "run_id": run_id,
                "workflow_id": workflow_id,
                "status": o.status,
                "output": o.output,
                "log": log_path.display().to_string(),
            })
            .to_string(),
        ),
        None => ToolResult::success(
            json!({
                "run_id": run_id,
                "workflow_id": workflow_id,
                "status": "running",
                "log": log_path.display().to_string(),
                "note": "still running past the wait budget — it continues in the background. \
                         Call `await_workflow` with this `run_id` to keep waiting, or move on.",
            })
            .to_string(),
        ),
    }
}

/// `run_workflow` — orchestrator-callable spawn + inline await of another
/// workflow.
pub struct RunWorkflowTool;

impl Default for RunWorkflowTool {
    fn default() -> Self {
        Self::new()
    }
}

impl RunWorkflowTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for RunWorkflowTool {
    fn name(&self) -> &str {
        RUN_WORKFLOW_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Run another workflow as a subagent and wait for its result, the way a \
         function call waits on its callee. Spawns the target workflow as a \
         fresh autonomous run (its own log + iteration budget), then waits up \
         to `wait_seconds` (default 90, max 600) for it to finish. If it \
         finishes in time you get back its terminal `status` (DONE / \
         DEGENERATE / FAILED) and `output`; if it outlives the wait it \
         auto-detaches and returns `status: \"running\"` plus a `run_id` you \
         can re-attach to with `await_workflow`. Pass `wait_seconds: 0` to \
         fire-and-forget (returns immediately with the `run_id`) — use that to \
         chain long-running workflows (e.g. after opening a PR, kick off \
         `pr-review-shepherd` and move on). Arguments: `workflow_id` (string, \
         required) names a workflow from `list_workflows`; `inputs` (object) is \
         the input map that workflow declares; `wait_seconds` (int, optional). \
         Errors (unknown workflow, missing required inputs, guardrail trip) come \
         back synchronously so you can correct and retry."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "workflow_id": {
                    "type": "string",
                    "description": "Id of the workflow to run (must appear in `list_workflows`)."
                },
                "inputs": {
                    "type": "object",
                    "description": "Input object passed to the workflow. Required keys are \
                                    declared by the target workflow's [[inputs]] block."
                },
                "wait_seconds": {
                    "type": "integer",
                    "description": "How long to wait inline for the result before auto-detaching \
                                    (default 90, max 600). 0 = fire-and-forget."
                }
            },
            "required": ["workflow_id"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Spawning another autonomous run carries the same blast radius as the
        // parent run that's calling it (background tokio task, no approval
        // gate). The parent is already inside an autonomous context, so gating
        // here would double-count — keep it ungated and let the target
        // workflow's definition govern what its run may do.
        PermissionLevel::None
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, tinytools::ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        _options: tinytools::ToolCallOptions,
        context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let dispatch = crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(context);
        let origin = crate::agent::tinyagents::host::run_context::turn_origin_for_tool(context);
        // Accept `workflow_id`, falling back to the legacy `skill_id` alias so
        // any in-flight caller from before the rename still works.
        let workflow_id = match args
            .get("workflow_id")
            .or_else(|| args.get("skill_id"))
            .and_then(|v| v.as_str())
        {
            Some(s) if !s.trim().is_empty() => s.to_string(),
            _ => {
                return Ok(ToolResult::error(
                    "run_workflow: missing required argument `workflow_id` (non-empty string)",
                ));
            }
        };
        let inputs = args.get("inputs").cloned();
        let wait_seconds = parse_wait_seconds(&args);

        // Fire-and-forget: only the spawn backstop applies — no await, so no
        // re-entrancy/nesting slot to take.
        if wait_seconds == 0 {
            return match spawn_workflow_run_background(
                &dispatch,
                workflow_id.clone(),
                inputs,
                origin,
            )
            .await
            {
                // Count only spawns that actually start against the backstop —
                // unknown-workflow / bad-input rejections (the Err arm) must not
                // burn the budget, or rejected calls accumulate and trip the
                // guard for legitimate ones.
                Ok(started) => {
                    if let Err(e) = guard::account_spawn(&dispatch) {
                        return Ok(ToolResult::error(format!("run_workflow: {e}")));
                    }
                    Ok(ToolResult::success(
                    json!({
                        "run_id": started.run_id,
                        "workflow_id": started.workflow_id,
                        "status": "started",
                        "log": started.log_path.display().to_string(),
                        "note": "fire-and-forget — runs independently to a terminal state. \
                                 Use `await_workflow` with this `run_id` if you want its result.",
                    })
                    .to_string(),
                    ))
                }
                Err(e) => Ok(ToolResult::error(format!("run_workflow: {e}"))),
            };
        }

        // Awaited path: take the re-entrancy/nesting slot first (so a tight
        // loop is rejected before we even spawn), then account the spawn —
        // but only once it actually starts, so a rejected spawn doesn't burn
        // the backstop.
        let _guard = match guard::acquire_await(&dispatch, reentrancy_key(&workflow_id, &inputs)) {
            Ok(g) => g,
            Err(e) => return Ok(ToolResult::error(format!("run_workflow: {e}"))),
        };

        let started =
            match spawn_workflow_run_background(&dispatch, workflow_id.clone(), inputs, origin)
                .await
            {
                Ok(s) => {
                    if let Err(e) = guard::account_spawn(&dispatch) {
                        return Ok(ToolResult::error(format!("run_workflow: {e}")));
                    }
                    s
                }
                Err(e) => return Ok(ToolResult::error(format!("run_workflow: {e}"))),
            };
        tracing::debug!(
            workflow_id = %started.workflow_id,
            run_id = %started.run_id,
            wait_seconds,
            "[run_workflow] spawned; awaiting result inline"
        );
        let outcome = await_run_outcome(
            &started.log_path,
            std::time::Duration::from_secs(wait_seconds),
        )
        .await;
        Ok(outcome_to_result(
            &started.run_id,
            &started.workflow_id,
            &started.log_path,
            outcome,
        ))
    }
}

/// `await_workflow` — re-attach to a detached run by `run_id` and wait.
pub struct AwaitWorkflowTool;

impl Default for AwaitWorkflowTool {
    fn default() -> Self {
        Self::new()
    }
}

impl AwaitWorkflowTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for AwaitWorkflowTool {
    fn name(&self) -> &str {
        AWAIT_WORKFLOW_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Re-attach to a workflow run you previously spawned (a `run_workflow` \
         call that returned `status: \"running\"` or `\"started\"`) and wait up \
         to `wait_seconds` (default 90, max 600) for it to finish. Returns its \
         terminal `status` + `output` if it lands in time, otherwise \
         `status: \"running\"` again so you can poll once more or move on. \
         Argument: `run_id` (string, required); `wait_seconds` (int, optional)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "run_id": {
                    "type": "string",
                    "description": "The `run_id` returned by an earlier `run_workflow` call."
                },
                "wait_seconds": {
                    "type": "integer",
                    "description": "How long to wait inline (default 90, max 600)."
                }
            },
            "required": ["run_id"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, tinytools::ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        _options: tinytools::ToolCallOptions,
        context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let dispatch = crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(context);
        let run_id = match args.get("run_id").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s.to_string(),
            _ => {
                return Ok(ToolResult::error(
                    "await_workflow: missing required argument `run_id` (non-empty string)",
                ));
            }
        };
        let wait_seconds = parse_wait_seconds(&args);

        let workspace = resolve_workspace_dir().await;
        let visible_run = crate::skills::run_log::scan_runs(&workspace, None, usize::MAX)
            .into_iter()
            .find(|run| run.run_id == run_id);
        let Some(visible_run) = visible_run else {
            return Ok(ToolResult::error(format!(
                "await_workflow: no run found for run_id `{run_id}` (it may not exist, is not \
                 available in this workspace, or hasn't started writing its log yet)"
            )));
        };
        let log_path = std::path::PathBuf::from(&visible_run.log_path);

        // Take an await slot so the LLM can't stack unbounded waits or
        // double-await the same run; keyed by run_id.
        let _guard = match guard::acquire_await(&dispatch, format!("await:{run_id}")) {
            Ok(g) => g,
            Err(e) => return Ok(ToolResult::error(format!("await_workflow: {e}"))),
        };

        let outcome =
            await_run_outcome(&log_path, std::time::Duration::from_secs(wait_seconds)).await;
        Ok(outcome_to_result(
            &run_id,
            &visible_run.workflow_id,
            &log_path,
            outcome,
        ))
    }
}

#[cfg(test)]
#[path = "run_workflow_tests.rs"]
mod tests;

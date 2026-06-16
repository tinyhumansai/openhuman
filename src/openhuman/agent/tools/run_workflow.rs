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
//! Guardrails (see the `guard` module): a process-lifetime spawn backstop,
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

use crate::openhuman::skill_runtime::{await_run_outcome, spawn_workflow_run_background};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use crate::openhuman::workflows::schemas::resolve_workspace_dir;

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
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{LazyLock, Mutex};

    /// Process-lifetime backstop against a runaway spawn loop.
    const TOTAL_SPAWN_BACKSTOP: u64 = 500;
    /// Max workflows being synchronously awaited at once. Because an awaiting
    /// call holds its slot for the whole nested wait, this also bounds the
    /// depth of synchronous workflow→workflow chains.
    const MAX_ACTIVE_AWAITS: u64 = 8;

    static TOTAL_SPAWNS: AtomicU64 = AtomicU64::new(0);
    static ACTIVE_AWAITS: AtomicU64 = AtomicU64::new(0);
    static ACTIVE_KEYS: LazyLock<Mutex<HashSet<String>>> =
        LazyLock::new(|| Mutex::new(HashSet::new()));

    /// RAII guard held while a call awaits a run. Dropping it frees the
    /// active-await slot and clears the re-entrancy key.
    pub struct AwaitGuard {
        key: String,
    }

    impl Drop for AwaitGuard {
        fn drop(&mut self) {
            ACTIVE_AWAITS.fetch_sub(1, Ordering::SeqCst);
            if let Ok(mut keys) = ACTIVE_KEYS.lock() {
                keys.remove(&self.key);
            }
        }
    }

    /// Account a spawn against the process-lifetime backstop. Returns `Err`
    /// once the cap trips. Called by both the awaited and the fire-and-forget
    /// paths so neither can loop forever.
    pub fn account_spawn() -> Result<(), String> {
        let n = TOTAL_SPAWNS.fetch_add(1, Ordering::SeqCst) + 1;
        if n > TOTAL_SPAWN_BACKSTOP {
            return Err(format!(
                "refused — process spawn backstop hit ({TOTAL_SPAWN_BACKSTOP} workflow runs \
                 spawned this session). This guards against a runaway spawn loop."
            ));
        }
        Ok(())
    }

    /// Test-only reader for the process-lifetime spawn counter. Used by the
    /// regression test that asserts a rejected spawn (e.g. unknown workflow
    /// id) doesn't consume a backstop slot.
    #[cfg(test)]
    pub fn total_spawns() -> u64 {
        TOTAL_SPAWNS.load(Ordering::SeqCst)
    }

    /// Acquire an await slot + re-entrancy lock for `key` (a workflow-id +
    /// inputs fingerprint, or `await:<run_id>` for re-attach). `Err` if too
    /// many awaits are in flight (nesting/fan-out cap) or the same key is
    /// already being awaited up the stack (re-entrant tight loop).
    pub fn acquire_await(key: String) -> Result<AwaitGuard, String> {
        let mut keys = ACTIVE_KEYS
            .lock()
            .map_err(|_| "internal guard lock poisoned".to_string())?;
        if keys.contains(&key) {
            return Err(
                "refused — this exact workflow + inputs is already being awaited higher up the \
                 call chain (re-entrant loop). Wait for it to finish or vary the inputs."
                    .to_string(),
            );
        }
        if ACTIVE_AWAITS.load(Ordering::SeqCst) >= MAX_ACTIVE_AWAITS {
            return Err(format!(
                "refused — {MAX_ACTIVE_AWAITS} workflows are already being awaited concurrently \
                 (nesting/fan-out cap). Let some finish, or spawn with `wait_seconds: 0` to \
                 detach instead of awaiting."
            ));
        }
        keys.insert(key.clone());
        ACTIVE_AWAITS.fetch_add(1, Ordering::SeqCst);
        Ok(AwaitGuard { key })
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
                sorted.sort_by_key(|(k, _)| k.clone());
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
    outcome: Option<crate::openhuman::workflows::run_log::RunOutcome>,
) -> ToolResult {
    match outcome {
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
pub struct RunWorkflowTool {
    /// Per-profile allowlist of runnable workflow `dir_name` slugs. `None`
    /// (the default) means every installed workflow may be run.
    skill_allowlist: Option<std::collections::HashSet<String>>,
}

impl Default for RunWorkflowTool {
    fn default() -> Self {
        Self::new()
    }
}

impl RunWorkflowTool {
    pub fn new() -> Self {
        Self {
            skill_allowlist: None,
        }
    }

    /// Restrict which workflows this tool may run to a per-profile allowlist.
    pub fn with_skill_allowlist(
        mut self,
        allowlist: Option<std::collections::HashSet<String>>,
    ) -> Self {
        self.skill_allowlist = allowlist;
        self
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
        if let Some(allow) = &self.skill_allowlist {
            if !allow.contains(&workflow_id) {
                log::debug!("[profiles] run_workflow blocked by profile allowlist: {workflow_id}");
                return Ok(ToolResult::error(format!(
                    "run_workflow: workflow `{workflow_id}` is not available to the active agent profile"
                )));
            }
        }
        let inputs = args.get("inputs").cloned();
        let wait_seconds = parse_wait_seconds(&args);

        // Fire-and-forget: only the spawn backstop applies — no await, so no
        // re-entrancy/nesting slot to take.
        if wait_seconds == 0 {
            return match spawn_workflow_run_background(workflow_id.clone(), inputs).await {
                // Count only spawns that actually start against the backstop —
                // unknown-workflow / bad-input rejections (the Err arm) must not
                // burn the budget, or rejected calls accumulate and trip the
                // guard for legitimate ones.
                Ok(started) => {
                    if let Err(e) = guard::account_spawn() {
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
        // the process backstop.
        let _guard = match guard::acquire_await(reentrancy_key(&workflow_id, &inputs)) {
            Ok(g) => g,
            Err(e) => return Ok(ToolResult::error(format!("run_workflow: {e}"))),
        };

        let started = match spawn_workflow_run_background(workflow_id.clone(), inputs).await {
            Ok(s) => {
                if let Err(e) = guard::account_spawn() {
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
        let log_path =
            match crate::openhuman::workflows::run_log::find_run_log_path(&workspace, &run_id) {
                Some(p) => p,
                None => {
                    return Ok(ToolResult::error(format!(
                        "await_workflow: no run found for run_id `{run_id}` (it may not exist or \
                         hasn't started writing its log yet)"
                    )));
                }
            };

        // Take an await slot so the LLM can't stack unbounded waits or
        // double-await the same run; keyed by run_id.
        let _guard = match guard::acquire_await(format!("await:{run_id}")) {
            Ok(g) => g,
            Err(e) => return Ok(ToolResult::error(format!("await_workflow: {e}"))),
        };

        let outcome =
            await_run_outcome(&log_path, std::time::Duration::from_secs(wait_seconds)).await;
        // workflow_id isn't carried on the handle here; the run_id is the
        // stable key the caller holds, so echo that and leave workflow_id blank.
        Ok(outcome_to_result(&run_id, "", &log_path, outcome))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_workflow_name_and_schema_basics() {
        let t = RunWorkflowTool::new();
        assert_eq!(t.name(), "run_workflow");
        let schema = t.parameters_schema();
        let required = schema
            .get("required")
            .and_then(|v| v.as_array())
            .expect("required array");
        assert!(required.iter().any(|v| v.as_str() == Some("workflow_id")));
        // inputs is optional now (a workflow may declare no inputs).
        assert!(!required.iter().any(|v| v.as_str() == Some("inputs")));
    }

    #[test]
    fn await_workflow_name_and_schema_basics() {
        let t = AwaitWorkflowTool::new();
        assert_eq!(t.name(), "await_workflow");
        let required = t
            .parameters_schema()
            .get("required")
            .and_then(|v| v.as_array())
            .cloned()
            .expect("required array");
        assert!(required.iter().any(|v| v.as_str() == Some("run_id")));
    }

    #[tokio::test]
    async fn run_workflow_missing_id_returns_tool_error_not_panic() {
        let t = RunWorkflowTool::new();
        let res = t
            .execute(json!({"inputs": {}}))
            .await
            .expect("Ok(ToolResult)");
        assert!(res.is_error, "expected ToolResult::error");
        assert!(res.output().contains("workflow_id"));
    }

    #[tokio::test]
    async fn await_workflow_missing_run_id_returns_tool_error() {
        let t = AwaitWorkflowTool::new();
        let res = t.execute(json!({})).await.expect("Ok(ToolResult)");
        assert!(res.is_error);
        assert!(res.output().contains("run_id"));
    }

    #[test]
    fn wait_seconds_defaults_and_clamps() {
        assert_eq!(parse_wait_seconds(&json!({})), DEFAULT_WAIT_SECONDS);
        assert_eq!(parse_wait_seconds(&json!({"wait_seconds": 5})), 5);
        assert_eq!(parse_wait_seconds(&json!({"wait_seconds": 0})), 0);
        assert_eq!(
            parse_wait_seconds(&json!({"wait_seconds": 99_999})),
            MAX_WAIT_SECONDS
        );
    }

    #[test]
    fn reentrancy_key_distinguishes_inputs() {
        let a = reentrancy_key("wf", &Some(json!({"pr": 1})));
        let b = reentrancy_key("wf", &Some(json!({"pr": 2})));
        let c = reentrancy_key("wf", &None);
        assert_ne!(a, b);
        assert_ne!(a, c);
        // Same id + same inputs → same key (so a tight loop is caught).
        assert_eq!(a, reentrancy_key("wf", &Some(json!({"pr": 1}))));
    }

    // ── Spawn-guard tests ────────────────────────────────────────────────
    //
    // The guards are process-global statics, so the await-slot tests share a
    // lock to avoid clobbering each other's `ACTIVE_AWAITS`/`ACTIVE_KEYS` count
    // under cargo's parallel runner. The RAII `AwaitGuard` frees its slot + key
    // on drop, so these leave no residue (unlike the spawn backstop, which is
    // monotonic by design — see its test).
    fn guard_serial() -> &'static std::sync::Mutex<()> {
        static L: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        L.get_or_init(|| std::sync::Mutex::new(()))
    }

    #[test]
    fn acquire_await_rejects_the_same_key_reentrantly() {
        let _s = guard_serial().lock().unwrap();
        let key = "reentry-test\u{1}null".to_string();
        let held = super::guard::acquire_await(key.clone()).expect("first acquire");
        let again = super::guard::acquire_await(key.clone());
        assert!(again.is_err(), "the same key while held must be rejected");
        assert!(again.err().unwrap().contains("re-entrant"));
        drop(held); // frees the key
        super::guard::acquire_await(key).expect("after drop the key is free again");
    }

    #[test]
    fn acquire_await_caps_concurrent_awaits() {
        let _s = guard_serial().lock().unwrap();
        // MAX_ACTIVE_AWAITS is 8; hold 8 distinct keys, then the 9th must reject.
        let mut held = Vec::new();
        for i in 0..8 {
            held.push(super::guard::acquire_await(format!("cap-test-{i}")).expect("under the cap"));
        }
        let ninth = super::guard::acquire_await("cap-test-9".to_string());
        assert!(ninth.is_err(), "the 9th concurrent await must reject");
        assert!(ninth.err().unwrap().contains("already being awaited"));
        // Free one slot → the next acquire succeeds.
        held.pop();
        super::guard::acquire_await("cap-test-9".to_string()).expect("a freed slot is reusable");
    }

    #[tokio::test]
    async fn unknown_workflow_id_does_not_burn_a_spawn_slot() {
        // Regression: a rejected spawn (unknown workflow id) must NOT consume a
        // slot against the process-lifetime backstop. `account_spawn` runs only
        // in the `Ok(started)` arm — after `spawn_workflow_run_background`
        // succeeds — so an unknown id (which fails synchronously) never accounts
        // a spawn. Without this ordering, an agent retrying a bad id would
        // exhaust the 500-spawn budget for legitimate runs. Asserts the counter
        // DELTA is zero (the counter is global + monotonic, so absolute value is
        // shared with the backstop test — hence the serial lock + delta check).
        let _s = guard_serial().lock().unwrap();
        let before = super::guard::total_spawns();
        let t = RunWorkflowTool::new();
        // wait_seconds: 0 → fire-and-forget path (no await slot taken); the
        // unknown id makes the spawn fail before accounting.
        let res = t
            .execute(json!({
                "workflow_id": "definitely-not-a-real-workflow-zzz",
                "wait_seconds": 0
            }))
            .await
            .expect("Ok(ToolResult)");
        assert!(res.is_error, "unknown workflow id must return a tool error");
        assert!(
            res.output().contains("unknown") || res.output().contains("workflow"),
            "error should reference the unknown workflow: {}",
            res.output()
        );
        let after = super::guard::total_spawns();
        assert_eq!(
            before, after,
            "a rejected spawn must not increment the spawn backstop counter"
        );
    }

    #[test]
    fn account_spawn_trips_the_process_backstop() {
        let _s = guard_serial().lock().unwrap();
        // TOTAL_SPAWN_BACKSTOP is 500 and the counter is process-global +
        // monotonic (no reset by design — it's a runaway-loop backstop). Drive
        // well past it and assert it trips. NOTE: this permanently trips the
        // backstop for the rest of the process, which is fine because no other
        // non-ignored test calls `account_spawn` (only the #[ignore] e2e run
        // path does, and that runs in a separate process).
        let mut last = Ok(());
        for _ in 0..600 {
            last = super::guard::account_spawn();
            if last.is_err() {
                break;
            }
        }
        let err = last.expect_err("the spawn backstop must trip within 600 accounted spawns");
        assert!(err.contains("backstop"), "got: {err}");
    }
}

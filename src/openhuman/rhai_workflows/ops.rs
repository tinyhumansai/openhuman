//! Cell evaluation orchestration: resolve the session, run the cell off the
//! async workers (`spawn_blocking`) under layered time bounds, forward
//! cancellation, and map every failure mode to a model-consumable result.

use std::sync::{Arc, TryLockError};
use std::time::Duration;

use tinyagents::registry::{CapabilityRegistry, ComponentKind};
use tinyagents::{
    ReplCallKind, ReplCallRecord, ReplCancelFlag, ReplCapabilities, ReplResult, ReplSession,
    TinyAgentsError,
};

use crate::openhuman::agent::harness::fork_context::current_parent;
use crate::openhuman::security::live_policy;
use crate::openhuman::security::policy::AutonomyLevel;
use crate::openhuman::tinyagents::run_cancellation_context::current_run_cancellation;

use super::bridge::build_capability_registry;
use super::policy::{resolve_policy, DEFAULT_RHAI_TIMEOUT_SECS};
use super::sessions::RhaiSessionManager;
use super::types::{RhaiCallSummary, RhaiEvalRequest, RhaiEvalResponse, RhaiLimitsRemaining};

/// Grace added to the inner policy timeout for the outer `spawn_blocking`
/// backstop — the inner deadline should always fire first; this defends against
/// bugs in the inner layers, and if it ever fires the session is dropped.
const OUTER_TIMEOUT_GRACE: Duration = Duration::from_secs(5);

/// Hard cap on the characters returned to the model in `stdout`, so a runaway
/// print cannot flood the context window even within the byte policy.
const MAX_RHAI_RESULT_CHARS: usize = 24_000;

/// A typed Rhai failure, each mapped to a model-consumable tool result by
/// [`RhaiError::message`] (with a fix hint) and [`RhaiError::kind`].
#[derive(Debug, Clone)]
pub(crate) enum RhaiError {
    /// The autonomy tier or policy refused the session.
    Denied(String),
    /// Script failed to compile or raised a runtime error.
    Script(String),
    /// A wall-clock deadline elapsed (script or in-flight capability call).
    Timeout(String),
    /// An op/output/script-size/call-count limit tripped.
    LimitExceeded(String),
    /// A script referenced an unknown tool/model/agent.
    UnknownCapability(String),
    /// Sub-agent recursion depth exceeded.
    Depth(String),
    /// A capability call itself returned an error.
    CapabilityError(String),
    /// The user cancelled the run.
    Cancelled,
    /// The session is already evaluating another cell.
    SessionBusy(String),
    /// An internal failure (join panic, poisoned session, missing context).
    Internal(String),
}

impl RhaiError {
    /// A stable, snake_case kind tag for logging and the tool result.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            RhaiError::Denied(_) => "denied",
            RhaiError::Script(_) => "script_error",
            RhaiError::Timeout(_) => "timeout",
            RhaiError::LimitExceeded(_) => "limit_exceeded",
            RhaiError::UnknownCapability(_) => "unknown_capability",
            RhaiError::Depth(_) => "recursion_depth",
            RhaiError::CapabilityError(_) => "capability_error",
            RhaiError::Cancelled => "cancelled",
            RhaiError::SessionBusy(_) => "session_busy",
            RhaiError::Internal(_) => "internal",
        }
    }

    /// The model-facing message, including a concrete fix hint.
    pub(crate) fn message(&self) -> String {
        match self {
            RhaiError::Denied(m) => m.clone(),
            RhaiError::Script(m) => {
                format!("rhai_workflows script error: {m}\nFix the script and retry — reuse the same session_id to keep your bindings.")
            }
            RhaiError::Timeout(m) => {
                format!("rhai_workflows cell timed out: {m}\nSplit the work across cells, lower fan-out, or raise timeout_secs.")
            }
            RhaiError::LimitExceeded(m) => {
                format!("rhai_workflows limit exceeded: {m}\nSplit the work across cells, or (full tier) raise the relevant limit in `limits`.")
            }
            RhaiError::UnknownCapability(m) => {
                format!("rhai_workflows unknown capability: {m}")
            }
            RhaiError::Depth(m) => format!("rhai_workflows recursion depth exceeded: {m}"),
            RhaiError::CapabilityError(m) => {
                format!("rhai_workflows capability call failed: {m}\nInspect the failing call's arguments and retry.")
            }
            RhaiError::Cancelled => {
                "rhai_workflows cell was cancelled by the user. The session is intact and resumable."
                    .to_string()
            }
            RhaiError::SessionBusy(m) => m.clone(),
            RhaiError::Internal(m) => format!("rhai_workflows internal error: {m}"),
        }
    }
}

/// The registered capability names, snapshotted for unknown-capability error
/// messages so the model sees the live surface it can actually call.
struct RhaiAvailable {
    tools: Vec<String>,
    agents: Vec<String>,
    models: Vec<String>,
}

impl RhaiAvailable {
    fn from_registry(registry: &CapabilityRegistry<()>) -> Self {
        Self {
            tools: registry.names(ComponentKind::Tool),
            agents: registry.names(ComponentKind::Agent),
            models: registry.names(ComponentKind::Model),
        }
    }
}

/// The internal result of the blocking cell task, distinguishing a lock
/// contention / poison from an actual evaluation outcome.
enum CellRun {
    /// The cell ran; carries the raw evaluation result.
    Completed(Result<ReplResult, TinyAgentsError>),
    /// Another cell holds the session lock.
    Busy,
    /// The session mutex was poisoned by a prior panic.
    Poisoned,
}

/// Evaluates one Rhai cell against a (possibly new) session, fail-closed.
///
/// Resolves the parent turn context, maps autonomy/timeout to a `ReplPolicy`,
/// resolves or creates the session, wires user cancellation to a fresh per-cell
/// flag, runs `eval_cell` on `spawn_blocking` under an outer `tokio::timeout`
/// backstop, and maps every outcome to [`RhaiEvalResponse`] or a typed
/// [`RhaiError`].
pub(crate) async fn eval_rhai_cell(req: RhaiEvalRequest) -> Result<RhaiEvalResponse, RhaiError> {
    let parent = current_parent().ok_or_else(|| {
        RhaiError::Internal(
            "no parent execution context — the rhai_workflows tool must run inside an agent turn"
                .to_string(),
        )
    })?;

    let tier = live_policy::current()
        .map(|p| p.autonomy)
        .unwrap_or(AutonomyLevel::Supervised);
    let policy =
        resolve_policy(tier, req.timeout_secs, req.limits.as_ref()).map_err(RhaiError::Denied)?;

    let registry = build_capability_registry(&parent);
    run_cell(
        req,
        policy,
        registry,
        &parent.session_id,
        RhaiSessionManager::global(),
    )
    .await
}

/// The parent-independent core of [`eval_rhai_cell`]: given a resolved policy and
/// a prebuilt capability registry, resolves/creates the session, runs the cell
/// under the layered time bounds and cancellation wiring, and maps the outcome.
///
/// Split out so the full evaluation path is unit-testable with a hand-built
/// registry, without constructing a whole `ParentExecutionContext`.
async fn run_cell(
    req: RhaiEvalRequest,
    policy: tinyagents::ReplPolicy,
    registry: CapabilityRegistry<()>,
    thread_scope: &str,
    manager: &RhaiSessionManager,
) -> Result<RhaiEvalResponse, RhaiError> {
    let session_id = req
        .session_id
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("rhai_workflows-{}", uuid::Uuid::new_v4()));
    let key = RhaiSessionManager::session_key(thread_scope, &session_id);

    // Snapshot the registry's names for error messages, then hand it to the
    // session builder (used only on a fresh session; a reused session keeps its
    // own registry).
    let available = RhaiAvailable::from_registry(&registry);
    let policy_for_build = policy.clone();

    tracing::info!(
        session_key = %key,
        thread_id = %thread_scope,
        session_id = %session_id,
        script_bytes = req.script.len(),
        "[rhai_workflows] eval_rhai_cell: start"
    );

    let handle = manager.get_or_create(&key, move || {
        let capabilities = ReplCapabilities::new(Arc::new(registry));
        ReplSession::<()>::new()
            .with_capabilities(capabilities)
            .with_policy(policy_for_build)
    });

    // Fresh per-cell cancel flag (sticky flags would leave a reused session
    // permanently cancelled), wired to the turn's run-cancellation token.
    let cell_flag = ReplCancelFlag::new();
    if let Some(token) = current_run_cancellation() {
        let flag = cell_flag.clone();
        tokio::spawn(async move {
            token.cancelled().await;
            tracing::debug!(
                "[rhai_workflows] run cancellation observed — tripping cell cancel flag"
            );
            flag.cancel();
        });
    }

    let session = handle.session.clone();
    let script = req.script.clone();
    let flag_for_cell = cell_flag.clone();
    let outer_bound = policy
        .timeout
        .unwrap_or(Duration::from_secs(DEFAULT_RHAI_TIMEOUT_SECS))
        + OUTER_TIMEOUT_GRACE;

    let join = tokio::task::spawn_blocking(move || {
        let mut guard = match session.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => return CellRun::Busy,
            Err(TryLockError::Poisoned(_)) => return CellRun::Poisoned,
        };
        // Install this cell's fresh cancel flag before running.
        guard.set_cancel_flag(flag_for_cell);
        CellRun::Completed(guard.eval_cell(&script))
    });

    let run = match tokio::time::timeout(outer_bound, join).await {
        Ok(Ok(run)) => run,
        Ok(Err(join_err)) => {
            // The blocking task panicked — the session may be poisoned; drop it.
            manager.close(&key);
            tracing::error!(session_key = %key, %join_err, "[rhai_workflows] cell task panicked — session dropped");
            return Err(RhaiError::Internal(format!(
                "rhai_workflows cell task failed: {join_err}"
            )));
        }
        Err(_elapsed) => {
            // The inner deadline should always fire first; if the outer backstop
            // fires, the blocking thread may still be unwinding — never reuse it.
            manager.close(&key);
            tracing::error!(
                session_key = %key,
                outer_secs = outer_bound.as_secs(),
                "[rhai_workflows] outer wall-clock backstop fired — session dropped (inner deadline should have fired first)"
            );
            return Err(RhaiError::Timeout(
                "the rhai_workflows cell exceeded its outer wall-clock backstop".to_string(),
            ));
        }
    };

    let eval = match run {
        CellRun::Completed(eval) => eval,
        CellRun::Busy => {
            tracing::info!(session_key = %key, "[rhai_workflows] session busy — concurrent cell rejected");
            return Err(RhaiError::SessionBusy(format!(
                "rhai_workflows session '{session_id}' is already evaluating a cell; wait for it to finish or use a different session_id"
            )));
        }
        CellRun::Poisoned => {
            manager.close(&key);
            return Err(RhaiError::Internal(
                "the rhai_workflows session was poisoned by a prior panic and has been dropped; start a fresh session".to_string(),
            ));
        }
    };

    let result = match eval {
        Ok(result) => result,
        Err(err) => {
            let mapped = map_eval_error(err, &available);
            tracing::info!(session_key = %key, kind = mapped.kind(), "[rhai_workflows] cell failed");
            return Err(mapped);
        }
    };

    let stats = manager.finish_cell(&key, &result).unwrap_or_default();
    if req.close_session {
        manager.close(&key);
    }

    let response = RhaiEvalResponse {
        session_id,
        stdout: truncate_chars(&result.stdout, MAX_RHAI_RESULT_CHARS),
        value: result.value.as_ref().map(|v| v.to_json()),
        variables_changed: result.variables_changed,
        calls: summarize_calls(&result.calls),
        final_answer: result.final_answer,
        elapsed_ms: result.elapsed.as_millis() as u64,
        cells_used: stats.cells,
        limits_remaining: RhaiLimitsRemaining {
            cells: policy.max_iterations.saturating_sub(stats.cells),
            model_calls: policy.max_model_calls.saturating_sub(stats.model_calls),
            tool_calls: policy.max_tool_calls.saturating_sub(stats.tool_calls),
            agent_calls: policy.max_agent_calls.saturating_sub(stats.agent_calls),
        },
        closed: req.close_session,
    };
    tracing::info!(
        session_key = %key,
        elapsed_ms = response.elapsed_ms,
        calls = response.calls.len(),
        cells_used = response.cells_used,
        "[rhai_workflows] eval_rhai_cell: done"
    );
    Ok(response)
}

/// Maps a raw `TinyAgentsError` from `eval_cell` onto a typed [`RhaiError`],
/// enriching unknown-capability errors with the live registered names.
fn map_eval_error(err: TinyAgentsError, available: &RhaiAvailable) -> RhaiError {
    match err {
        TinyAgentsError::Validation(m) => RhaiError::Script(m),
        TinyAgentsError::Timeout(m) => RhaiError::Timeout(m),
        TinyAgentsError::LimitExceeded(m) => RhaiError::LimitExceeded(m),
        TinyAgentsError::Cancelled => RhaiError::Cancelled,
        TinyAgentsError::SubAgentDepth(n) => RhaiError::Depth(format!(
            "sub-agent recursion exceeded the maximum depth of {n}"
        )),
        TinyAgentsError::ModelNotFound(name) => RhaiError::UnknownCapability(format!(
            "model `{name}` is not registered. Available models: {}",
            join_or_none(&available.models)
        )),
        TinyAgentsError::ToolNotFound(name) => RhaiError::UnknownCapability(format!(
            "tool `{name}` is not registered. Available tools: {}",
            join_or_none(&available.tools)
        )),
        TinyAgentsError::Capability(m) => RhaiError::UnknownCapability(format!(
            "{m}. Available agents: {}",
            join_or_none(&available.agents)
        )),
        TinyAgentsError::Tool(m) => RhaiError::CapabilityError(m),
        other => RhaiError::Script(other.to_string()),
    }
}

/// Summarizes recorded calls into the model-visible shape — kind, name, timing,
/// success only, never raw arguments/payloads.
fn summarize_calls(calls: &[ReplCallRecord]) -> Vec<RhaiCallSummary> {
    calls
        .iter()
        .map(|c| RhaiCallSummary {
            kind: call_kind_str(c.kind).to_string(),
            name: c.name.clone(),
            elapsed_ms: c.elapsed.as_millis() as u64,
            ok: true,
        })
        .collect()
}

fn call_kind_str(kind: ReplCallKind) -> &'static str {
    match kind {
        ReplCallKind::Model => "model",
        ReplCallKind::Tool => "tool",
        ReplCallKind::Graph => "graph",
        ReplCallKind::Agent => "agent",
        ReplCallKind::Emit => "emit",
    }
}

/// Renders a name list for an error message, or a placeholder when empty.
fn join_or_none(names: &[String]) -> String {
    if names.is_empty() {
        "(none)".to_string()
    } else {
        names.join(", ")
    }
}

/// Truncates `s` to at most `max` characters (char-boundary safe), appending a
/// marker when it had to cut.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str("\n… (rhai_workflows output truncated)");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tinyagents::harness::tool::{
        Tool as TaTool, ToolCall as TaToolCall, ToolResult as TaToolResult, ToolSchema,
    };
    use tinyagents::registry::CapabilityRegistry;
    use tinyagents::ReplPolicy;

    // ── Pure-function tests (mapping / helpers) ──────────────────────────────

    #[test]
    fn truncate_bounds_output() {
        let s = "a".repeat(100);
        let out = truncate_chars(&s, 10);
        assert!(out.starts_with(&"a".repeat(10)));
        assert!(out.contains("truncated"));
        assert!(truncate_chars("short", 10) == "short");
    }

    #[test]
    fn error_kinds_are_stable() {
        assert_eq!(RhaiError::Cancelled.kind(), "cancelled");
        assert_eq!(RhaiError::Timeout("x".into()).kind(), "timeout");
        assert!(RhaiError::Script("boom".into())
            .message()
            .contains("Fix the script"));
    }

    #[test]
    fn every_taxonomy_variant_maps_to_the_right_kind() {
        let avail = RhaiAvailable {
            tools: vec!["read_file".into(), "grep".into()],
            agents: vec!["researcher".into()],
            models: vec!["chat".into()],
        };
        let cases: Vec<(TinyAgentsError, &str)> = vec![
            (TinyAgentsError::Validation("parse".into()), "script_error"),
            (TinyAgentsError::Timeout("t".into()), "timeout"),
            (TinyAgentsError::LimitExceeded("l".into()), "limit_exceeded"),
            (TinyAgentsError::Cancelled, "cancelled"),
            (TinyAgentsError::SubAgentDepth(3), "recursion_depth"),
            (
                TinyAgentsError::ModelNotFound("m".into()),
                "unknown_capability",
            ),
            (
                TinyAgentsError::ToolNotFound("x".into()),
                "unknown_capability",
            ),
            (
                TinyAgentsError::Capability("agent nope".into()),
                "unknown_capability",
            ),
            (TinyAgentsError::Tool("boom".into()), "capability_error"),
        ];
        for (err, expected) in cases {
            assert_eq!(map_eval_error(err, &avail).kind(), expected);
        }
        // Unknown-tool errors list the live tool names for the model.
        let msg = map_eval_error(TinyAgentsError::ToolNotFound("nope".into()), &avail).message();
        assert!(msg.contains("read_file") && msg.contains("grep"), "{msg}");
    }

    // ── End-to-end `run_cell` matrix (with a hand-built registry) ────────────

    struct EchoTool;

    #[async_trait::async_trait]
    impl TaTool<()> for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "echoes its msg argument"
        }
        fn schema(&self) -> ToolSchema {
            ToolSchema {
                name: "echo".into(),
                description: "echoes".into(),
                parameters: serde_json::json!({ "type": "object" }),
                format: Default::default(),
            }
        }
        async fn call(&self, _s: &(), call: TaToolCall) -> tinyagents::Result<TaToolResult> {
            let msg = call
                .arguments
                .get("msg")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Ok(TaToolResult::text(
                call.id,
                call.name,
                format!("echo:{msg}"),
            ))
        }
    }

    fn echo_registry() -> CapabilityRegistry<()> {
        let mut registry = CapabilityRegistry::<()>::new();
        registry
            .register_tool(Arc::new(EchoTool))
            .expect("register echo");
        registry
    }

    fn req(script: &str) -> RhaiEvalRequest {
        RhaiEvalRequest {
            script: script.to_string(),
            ..Default::default()
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn happy_path_runs_a_tool_call_and_reports_usage() {
        let manager = RhaiSessionManager::new_for_test();
        let resp = run_cell(
            req(r#"tool_call(#{ tool: "echo", arguments: #{ msg: "hi" } })"#),
            ReplPolicy::default(),
            echo_registry(),
            "t",
            &manager,
        )
        .await
        .expect("cell ok");
        assert_eq!(resp.cells_used, 1);
        assert_eq!(resp.calls.len(), 1);
        assert_eq!(resp.calls[0].kind, "tool");
        assert_eq!(resp.calls[0].name, "echo");
        assert_eq!(
            resp.limits_remaining.tool_calls,
            ReplPolicy::default().max_tool_calls - 1
        );
        assert!(!resp.session_id.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn parse_error_is_a_script_error() {
        let manager = RhaiSessionManager::new_for_test();
        let err = run_cell(
            req("let x = ;"),
            ReplPolicy::default(),
            echo_registry(),
            "t",
            &manager,
        )
        .await
        .expect_err("parse error");
        assert_eq!(err.kind(), "script_error");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unknown_tool_lists_registered_names() {
        let manager = RhaiSessionManager::new_for_test();
        let err = run_cell(
            req(r#"tool_call(#{ tool: "does_not_exist" })"#),
            ReplPolicy::default(),
            echo_registry(),
            "t",
            &manager,
        )
        .await
        .expect_err("unknown tool");
        assert_eq!(err.kind(), "unknown_capability");
        assert!(err.message().contains("echo"), "{}", err.message());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn call_count_limit_fails_closed() {
        let manager = RhaiSessionManager::new_for_test();
        let policy = ReplPolicy {
            max_tool_calls: 1,
            ..ReplPolicy::default()
        };
        let err = run_cell(
            req(r#"tool_call(#{tool:"echo"}); tool_call(#{tool:"echo"})"#),
            policy,
            echo_registry(),
            "t",
            &manager,
        )
        .await
        .expect_err("limit");
        assert_eq!(err.kind(), "limit_exceeded");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn script_loop_times_out_within_the_bound() {
        let manager = RhaiSessionManager::new_for_test();
        let policy = ReplPolicy {
            timeout: Some(Duration::from_millis(200)),
            max_operations: 0,
            ..ReplPolicy::default()
        };
        let start = std::time::Instant::now();
        let err = run_cell(req("loop {}"), policy, echo_registry(), "t", &manager)
            .await
            .expect_err("timeout");
        assert_eq!(err.kind(), "timeout");
        assert!(
            start.elapsed() < Duration::from_secs(4),
            "took {:?}",
            start.elapsed()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn oversized_output_fails_closed() {
        let manager = RhaiSessionManager::new_for_test();
        let policy = ReplPolicy {
            max_output_bytes: 64,
            ..ReplPolicy::default()
        };
        let err = run_cell(
            req(r#"for i in 0..1000 { print("xxxxxxxxxxxxxxxx"); }"#),
            policy,
            echo_registry(),
            "t",
            &manager,
        )
        .await
        .expect_err("output limit");
        assert_eq!(err.kind(), "limit_exceeded");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn namespace_persists_and_close_session_drops_it() {
        let manager = RhaiSessionManager::new_for_test();
        let first = run_cell(
            RhaiEvalRequest {
                script: "let acc = 10; acc".into(),
                session_id: Some("keep".into()),
                ..Default::default()
            },
            ReplPolicy::default(),
            echo_registry(),
            "t",
            &manager,
        )
        .await
        .expect("cell 1");
        assert_eq!(first.session_id, "keep");

        let second = run_cell(
            RhaiEvalRequest {
                script: "acc + 5".into(),
                session_id: Some("keep".into()),
                close_session: true,
                ..Default::default()
            },
            ReplPolicy::default(),
            echo_registry(),
            "t",
            &manager,
        )
        .await
        .expect("cell 2");
        assert_eq!(second.value, Some(serde_json::json!(15)));
        assert!(second.closed);
        assert_eq!(manager.len(), 0, "close_session dropped the session");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn concurrent_cell_on_a_busy_session_is_rejected() {
        let manager = RhaiSessionManager::new_for_test();
        let key = RhaiSessionManager::session_key("t", "busy");
        let handle = manager.get_or_create(&key, || tinyagents::ReplSession::<()>::new());
        // Hold the session lock to simulate an in-flight cell.
        let _guard = handle.session.lock().unwrap();

        let res = run_cell(
            RhaiEvalRequest {
                script: "1".into(),
                session_id: Some("busy".into()),
                ..Default::default()
            },
            ReplPolicy::default(),
            CapabilityRegistry::new(),
            "t",
            &manager,
        )
        .await;
        assert!(matches!(res, Err(RhaiError::SessionBusy(_))), "got {res:?}");
    }
}

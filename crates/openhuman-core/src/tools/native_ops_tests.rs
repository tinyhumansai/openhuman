//! A direct tool run (`execute_tool`: the JS and RPC runs, workflow `tool_call`
//! nodes) passes the same security gate a harness turn installs. The gate under
//! test is built from the production pieces (`ModuleVerdicts` deriving the module's
//! `Check::HostAction`, `TinySecurityPolicy`, the harness gate); only the module is
//! scripted, behind the seam `ModuleVerdicts` already uses.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::json;
use tinyagents_harness::context::RunContext;
use tinyinference_llm::tool::ToolCall as TaToolCall;
use tinysecurity_bus::{Check, DenialReason, ToolPermission, Verdict};
use tinytools::PermissionLevel;

use crate::agent::tinyagents::middleware::approval::ApprovalSecurityMiddleware;
use crate::agent::tinyagents::middleware::security_gate::gate;
use crate::agent::tinyagents::middleware::tinysecurity::TinySecurityPolicy;
use crate::agent::tinyagents::middleware::tinysecurity_module::{ModuleAsk, ModuleVerdicts};

/// A tool that counts how often it really runs.
struct Counting {
    runs: Arc<AtomicUsize>,
    permission: PermissionLevel,
    fails: bool,
}

#[async_trait]
impl Tool for Counting {
    fn name(&self) -> &str {
        "write_something"
    }
    fn description(&self) -> &str {
        "test tool"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object" })
    }
    fn permission_level(&self) -> PermissionLevel {
        self.permission
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        if self.fails {
            anyhow::bail!("disk on fire");
        }
        Ok(ToolResult::success("wrote it"))
    }
}

struct Module {
    answer: Result<Verdict, String>,
    asked: Mutex<Vec<Vec<Check>>>,
    origins: Mutex<Vec<Option<crate::security::turn_origin::AgentTurnOrigin>>>,
}

#[async_trait]
impl ModuleAsk for Module {
    async fn ask(
        &self,
        ctx: &RunContext<OpenHumanRunContext>,
        _call: &TaToolCall,
        checks: Vec<Check>,
        _redact: bool,
    ) -> Result<Verdict, String> {
        self.asked.lock().unwrap().push(checks);
        self.origins.lock().unwrap().push(ctx.data.origin.clone());
        self.answer.clone()
    }
}

struct Run {
    outcome: Result<ExecuteToolOutcome, String>,
    runs: usize,
    asked: Vec<Vec<Check>>,
}

async fn direct(answer: Result<Verdict, String>, permission: PermissionLevel, fails: bool) -> Run {
    let runs = Arc::new(AtomicUsize::new(0));
    let tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![Box::new(Counting {
        runs: runs.clone(),
        permission,
        fails,
    })]);
    let module = Arc::new(Module {
        answer,
        asked: Mutex::new(Vec::new()),
        origins: Mutex::new(Vec::new()),
    });
    let policy = TinySecurityPolicy::new(Arc::new(ModuleVerdicts::with_module(
        vec![tools.clone()],
        module.clone(),
    )));
    let resolver = Arc::new(ApprovalSecurityMiddleware::new(vec![tools.clone()]));
    let outcome = run_gated(
        &tools,
        Arc::new(gate(Arc::new(policy), resolver)),
        "write_something",
        json!({"path": "a"}),
        false,
    )
    .await;
    let asked = module.asked.lock().unwrap().clone();
    Run {
        outcome,
        runs: runs.load(Ordering::SeqCst),
        asked,
    }
}

#[tokio::test]
async fn a_direct_run_the_policy_refuses_never_executes_the_tool() {
    let run = direct(
        Ok(Verdict::Deny {
            reason: DenialReason::ReadOnlyMode,
        }),
        PermissionLevel::Write,
        false,
    )
    .await;
    assert_eq!(run.runs, 0, "a refused tool must not run");
    let outcome = run.outcome.expect("the refusal is the tool result");
    assert!(outcome.result.is_error);
    assert!(
        outcome.result.output().contains("read-only"),
        "the model is told why: {}",
        outcome.result.output()
    );
}

#[tokio::test]
async fn a_direct_run_is_judged_as_an_acting_call_that_spends_one_action() {
    let run = direct(Ok(Verdict::Allow), PermissionLevel::Write, false).await;
    assert_eq!(run.runs, 1);
    assert_eq!(run.asked.len(), 1, "one policy question per direct run");
    let [Check::HostAction {
        permission,
        external_effect,
        spend,
        ..
    }] = run.asked[0].as_slice()
    else {
        panic!("expected one HostAction check, got {:?}", run.asked);
    };
    assert_eq!(*permission, ToolPermission::Write);
    assert!(!external_effect);
    assert!(*spend, "an acting call reserves its action at the gate");
    assert!(!run.outcome.unwrap().result.is_error);
}

#[tokio::test]
async fn an_exhausted_budget_refuses_a_direct_dangerous_run() {
    // A fault is a denial, never an allow: nothing ran.
    let run = direct(
        Ok(Verdict::Deny {
            reason: DenialReason::RateLimited { max_per_hour: 3 },
        }),
        PermissionLevel::Dangerous,
        false,
    )
    .await;
    assert_eq!(run.runs, 0);
    assert!(run
        .outcome
        .unwrap()
        .result
        .output()
        .contains("3 actions/hour"));
}

#[tokio::test]
async fn an_allowed_tool_failure_reaches_the_caller_as_an_error() {
    let run = direct(Ok(Verdict::Allow), PermissionLevel::Write, true).await;
    assert_eq!(run.runs, 1);
    let error = run.outcome.expect_err("the tool failed");
    assert!(error.contains("write_something"), "{error}");
    assert!(error.contains("disk on fire"), "{error}");
}

#[tokio::test]
async fn an_unknown_tool_is_reported_without_asking_anything() {
    let tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(Vec::new());
    let module = Arc::new(Module {
        answer: Ok(Verdict::Allow),
        asked: Mutex::new(Vec::new()),
        origins: Mutex::new(Vec::new()),
    });
    let policy = TinySecurityPolicy::new(Arc::new(ModuleVerdicts::with_module(
        vec![tools.clone()],
        module.clone(),
    )));
    let resolver = Arc::new(ApprovalSecurityMiddleware::new(vec![tools.clone()]));
    let error = run_gated(
        &tools,
        Arc::new(gate(Arc::new(policy), resolver)),
        "nope",
        json!({}),
        false,
    )
    .await
    .expect_err("unknown tool");
    assert!(error.contains("unknown tool"), "{error}");
    assert!(module.asked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn direct_tool_dispatch_preserves_its_host_turn_origin_through_the_gate() {
    let tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![Box::new(Counting {
        runs: Arc::new(AtomicUsize::new(0)),
        permission: PermissionLevel::Write,
        fails: false,
    })]);
    let module = Arc::new(Module {
        answer: Ok(Verdict::Allow),
        asked: Mutex::new(Vec::new()),
        origins: Mutex::new(Vec::new()),
    });
    let policy = TinySecurityPolicy::new(Arc::new(ModuleVerdicts::with_module(
        vec![tools.clone()],
        module.clone(),
    )));
    let resolver = Arc::new(ApprovalSecurityMiddleware::new(vec![tools.clone()]));
    let context =
        OpenHumanRunContext::new().with_origin(crate::security::turn_origin::AgentTurnOrigin::Cli);

    run_gated_with_context(
        &tools,
        Arc::new(gate(Arc::new(policy), resolver)),
        "write_something",
        json!({"path": "a"}),
        false,
        context,
    )
    .await
    .expect("the module allows the host action");

    let origins = module.origins.lock().unwrap();
    assert_eq!(origins.len(), 1);
    assert!(matches!(
        origins[0],
        Some(crate::security::turn_origin::AgentTurnOrigin::Cli)
    ));
}

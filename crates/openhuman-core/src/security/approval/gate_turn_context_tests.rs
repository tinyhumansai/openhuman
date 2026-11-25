//! The turn's approval context reaches the gate through the harness approval
//! resolver (`ApprovalSecurityMiddleware`), carried by the run context rather than
//! any ambient scope; a detached turn starts without it; concurrent turns keep
//! their own.

use super::*;
use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::agent::tinyagents::middleware::approval::ApprovalSecurityMiddleware;
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use tinyagents_harness::context::{RunConfig, RunContext};
use tinyagents_harness::middleware::{
    ApprovalExecutionOutcome, ApprovalGateMiddleware, ApprovalResolution, ApprovalResolver,
    BoxToolFuture, MiddlewareStack, ToolBaseCall,
};
use tinyinference_llm::tool::ToolCall as TaToolCall;
use tinytools::{PermissionLevel, Tool, ToolResult};

struct Effectful;

#[async_trait]
impl Tool for Effectful {
    fn name(&self) -> &str {
        "slack_post"
    }
    fn description(&self) -> &str {
        "test tool with an external effect"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    fn external_effect(&self) -> bool {
        true
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        unreachable!("the security gate never executes a tool")
    }
}

/// One turn's resolver and the run context it parks under.
struct TurnResolver {
    resolver: ApprovalSecurityMiddleware,
    ctx: RunContext<OpenHumanRunContext>,
}

impl TurnResolver {
    async fn park_for_approval_on(
        &self,
        gate: &ApprovalGate,
        call: &TaToolCall,
    ) -> ApprovalResolution {
        self.resolver.resolve_on(gate, &self.ctx, call).await
    }
}

fn security_gate_for(run: &OpenHumanRunContext) -> TurnResolver {
    TurnResolver {
        resolver: ApprovalSecurityMiddleware::new(vec![std::sync::Arc::new(vec![
            Box::new(Effectful) as Box<dyn Tool>,
        ])]),
        ctx: RunContext::new(RunConfig::new("gate-turn-context"), run.clone()),
    }
}

fn chat(thread: &str) -> ApprovalChatContext {
    ApprovalChatContext {
        thread_id: thread.to_string(),
        client_id: format!("client-{thread}"),
        request_id: Some(format!("req-{thread}")),
    }
}

fn web_origin_for(thread: &str) -> AgentTurnOrigin {
    AgentTurnOrigin::WebChat {
        thread_id: thread.to_string(),
        client_id: format!("client-{thread}"),
        request_id: Some(format!("req-{thread}")),
    }
}

fn call() -> TaToolCall {
    TaToolCall {
        id: "c1".into(),
        name: "slack_post".into(),
        arguments: serde_json::json!({"text": "hi"}),
        invalid: None,
    }
}

struct GateBackedResolver {
    gate: Arc<ApprovalGate>,
    security: ApprovalSecurityMiddleware,
}

#[async_trait]
impl ApprovalResolver<OpenHumanRunContext> for GateBackedResolver {
    async fn requires_approval(
        &self,
        ctx: &RunContext<OpenHumanRunContext>,
        call: &TaToolCall,
    ) -> bool {
        self.security
            .requires_approval(&ctx.data.dispatch_ctx(), &call.name, &call.arguments)
            .await
    }

    async fn resolve(
        &self,
        ctx: &RunContext<OpenHumanRunContext>,
        call: &TaToolCall,
    ) -> ApprovalResolution {
        self.security.resolve_on(&self.gate, ctx, call).await
    }

    fn record(&self, ticket: &str, result: &ToolResult) {
        self.record_terminal(ticket, ApprovalExecutionOutcome::Result(result));
    }

    fn record_terminal(&self, ticket: &str, outcome: ApprovalExecutionOutcome<'_>) {
        ApprovalSecurityMiddleware::record_terminal_on(&self.gate, ticket, outcome);
    }
}

struct CountingWrite {
    effects: Arc<AtomicUsize>,
}

impl ToolBaseCall<(), OpenHumanRunContext> for CountingWrite {
    fn call<'a>(
        &'a self,
        _ctx: &'a RunContext<OpenHumanRunContext>,
        _state: &'a (),
        _call: TaToolCall,
    ) -> BoxToolFuture<'a> {
        Box::pin(async move {
            self.effects.fetch_add(1, Ordering::SeqCst);
            Ok(ToolResult::success("posted"))
        })
    }
}

/// Waits for the thread's park to register, returning its request id.
async fn parked(gate: &ApprovalGate, thread: &str) -> String {
    for _ in 0..2_000 {
        if let Some(request_id) = gate.pending_for_thread(thread) {
            return request_id;
        }
        tokio::task::yield_now().await;
    }
    panic!("no approval parked for thread {thread}");
}

#[tokio::test]
async fn a_turn_parks_its_approval_on_the_chat_its_run_context_carries() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    let mut run = OpenHumanRunContext::new();
    run.approval = ApprovalTurnContext::chat(chat("t-turn"));
    let security = security_gate_for(&run);

    let g = gate.clone();
    let task = tokio::spawn(async move {
        turn_origin::with_origin(web_origin_for("t-turn"), async move {
            // A real await point before the gate check: the context must survive it.
            tokio::task::yield_now().await;
            security.park_for_approval_on(&g, &call()).await
        })
        .await
    });

    let request_id = parked(&gate, "t-turn").await;
    // A park with a chat context is attached to the turn, not a detached card.
    assert!(!gate.request_is_detached(&request_id));
    let pending = gate
        .list_pending()
        .unwrap()
        .into_iter()
        .find(|pending| pending.request_id == request_id)
        .expect("the parked approval is persisted");
    assert_eq!(pending.tool_call_id.as_deref(), Some("c1"));
    gate.decide(&request_id, ApprovalDecision::ApproveOnce)
        .unwrap();
    assert!(matches!(
        task.await.unwrap(),
        ApprovalResolution::Allow { .. }
    ));
}

#[tokio::test]
async fn a_detached_turn_has_no_chat_and_routes_through_its_origin_marked_detached() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    let mut parent = OpenHumanRunContext::new();
    parent.approval = ApprovalTurnContext::chat(chat("t-parent"));
    // What an async-delegated sub-agent runs with (#5499).
    let detached = parent.detached_child();
    let security = security_gate_for(&detached);

    let g = gate.clone();
    let task = tokio::spawn(async move {
        turn_origin::with_origin(web_origin_for("t-parent"), async move {
            security.park_for_approval_on(&g, &call()).await
        })
        .await
    });

    let request_id = parked(&gate, "t-parent").await;
    assert!(
        gate.request_is_detached(&request_id),
        "a detached sub-agent's park must not inherit the parent's chat context"
    );
    gate.decide(&request_id, ApprovalDecision::ApproveOnce)
        .unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn concurrent_turns_park_on_their_own_chats() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    let mut tasks = Vec::new();
    for thread in ["t-one", "t-two"] {
        let mut run = OpenHumanRunContext::new();
        run.approval = ApprovalTurnContext::chat(chat(thread));
        let security = security_gate_for(&run);
        let g = gate.clone();
        tasks.push(tokio::spawn(async move {
            turn_origin::with_origin(web_origin_for(thread), async move {
                security.park_for_approval_on(&g, &call()).await
            })
            .await
        }));
    }
    for thread in ["t-one", "t-two"] {
        let request_id = parked(&gate, thread).await;
        gate.decide(&request_id, ApprovalDecision::ApproveOnce)
            .unwrap();
    }
    for task in tasks {
        assert!(matches!(
            task.await.unwrap(),
            ApprovalResolution::Allow { .. }
        ));
    }
}

#[tokio::test]
async fn supervised_write_runs_once_after_approval_and_records_the_exact_call() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    let mut run = OpenHumanRunContext::new();
    run.approval = ApprovalTurnContext::chat(chat("t-supervised-write"));
    let ctx = RunContext::new(RunConfig::new("supervised-write"), run);
    let resolver = Arc::new(GateBackedResolver {
        gate: gate.clone(),
        security: ApprovalSecurityMiddleware::new(vec![Arc::new(vec![
            Box::new(Effectful) as Box<dyn Tool>
        ])]),
    });
    let mut stack: MiddlewareStack<(), OpenHumanRunContext> = MiddlewareStack::new();
    stack.push_tool_middleware(Arc::new(ApprovalGateMiddleware::new("approval", resolver)));
    let effects = Arc::new(AtomicUsize::new(0));
    let base = CountingWrite {
        effects: effects.clone(),
    };
    let mut supervised_call = call();
    supervised_call.id = "call-supervised-write".into();

    let task = tokio::spawn(async move {
        turn_origin::with_origin(web_origin_for("t-supervised-write"), async move {
            stack
                .run_wrapped_tool(&ctx, &(), supervised_call, &base)
                .await
                .unwrap()
                .into_result()
        })
        .await
    });

    let request_id = parked(&gate, "t-supervised-write").await;
    let pending = gate
        .list_pending()
        .unwrap()
        .into_iter()
        .find(|pending| pending.request_id == request_id)
        .expect("the supervised write is persisted while parked");
    assert_eq!(
        pending.tool_call_id.as_deref(),
        Some("call-supervised-write")
    );
    assert_eq!(
        effects.load(Ordering::SeqCst),
        0,
        "a parked write has no effect"
    );

    gate.decide(&request_id, ApprovalDecision::ApproveOnce)
        .unwrap();
    let result = task.await.unwrap();
    assert!(
        !result.is_error,
        "approved write should complete: {}",
        result.output()
    );
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    assert_eq!(
        stub_of(&gate).state(&request_id),
        Some(crate::security::approval::engine_stub::State::Executed(
            ExecutionOutcome::Success
        ))
    );
}

#[tokio::test]
async fn terminal_audit_failure_preserves_completed_result_and_denies_later_writes() {
    let (mut gate, _dir) = test_gate();
    gate.config.autonomy.auto_approve_all = true;
    let gate = Arc::new(gate);
    stub_of(&gate).fail_record_execution(true);
    let mut run = OpenHumanRunContext::new();
    run.approval = ApprovalTurnContext::chat(chat("t-audit-failure"));
    let ctx = RunContext::new(RunConfig::new("audit-failure"), run);
    let resolver = Arc::new(GateBackedResolver {
        gate: gate.clone(),
        security: ApprovalSecurityMiddleware::new(vec![Arc::new(vec![
            Box::new(Effectful) as Box<dyn Tool>
        ])]),
    });
    let mut stack: MiddlewareStack<(), OpenHumanRunContext> = MiddlewareStack::new();
    stack.push_tool_middleware(Arc::new(ApprovalGateMiddleware::new("approval", resolver)));
    let effects = Arc::new(AtomicUsize::new(0));
    let base = CountingWrite {
        effects: effects.clone(),
    };

    let mut first = call();
    first.id = "call-audit-fails-after-execution".into();
    let first_result = turn_origin::with_origin(web_origin_for("t-audit-failure"), async {
        stack
            .run_wrapped_tool(&ctx, &(), first, &base)
            .await
            .unwrap()
            .into_result()
    })
    .await;
    assert!(
        !first_result.is_error,
        "audit failure must not rewrite an executed result"
    );
    assert_eq!(effects.load(Ordering::SeqCst), 1);

    let mut second = call();
    second.id = "call-after-audit-failure".into();
    let second_result = turn_origin::with_origin(web_origin_for("t-audit-failure"), async {
        stack
            .run_wrapped_tool(&ctx, &(), second, &base)
            .await
            .unwrap()
            .into_result()
    })
    .await;
    assert!(
        second_result.is_error,
        "later external write must fail closed"
    );
    assert_eq!(effects.load(Ordering::SeqCst), 1);
}

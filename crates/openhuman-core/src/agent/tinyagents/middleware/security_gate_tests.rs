use super::*;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use async_trait::async_trait;
use tinyagents_harness::context::{RunConfig, RunContext};
use tinyagents_harness::middleware::{
    ApprovalResolution, ApprovalResolver, BoxToolFuture, MiddlewareStack, PolicyDecision,
    ToolBaseCall, ToolCallPolicy,
};
use tinyinference_llm::tool::ToolCall as TaToolCall;
use tinysecurity_bus::{DenialReason, Verdict};
use tinytools::ToolResult;

use super::super::tinysecurity::{TinySecurityPolicy, VerdictSource};

struct Module {
    answer: Result<Verdict, String>,
    asked: AtomicUsize,
}

struct BuilderPolicy(PolicyDecision);

#[async_trait]
impl ToolCallPolicy<OpenHumanRunContext> for BuilderPolicy {
    fn name(&self) -> &str {
        "test_builder_policy"
    }

    async fn check(
        &self,
        _ctx: &RunContext<OpenHumanRunContext>,
        _call: &TaToolCall,
    ) -> PolicyDecision {
        self.0.clone()
    }
}

#[async_trait]
impl VerdictSource for Module {
    async fn verdict(
        &self,
        _ctx: &RunContext<OpenHumanRunContext>,
        _call: &TaToolCall,
    ) -> Result<Verdict, String> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.answer.clone()
    }
}

#[derive(Default)]
struct Human {
    answer: Mutex<Option<ApprovalResolution>>,
    asked: AtomicUsize,
    recorded: Mutex<Vec<String>>,
}

#[async_trait]
impl ApprovalResolver<OpenHumanRunContext> for Human {
    async fn requires_approval(
        &self,
        _ctx: &RunContext<OpenHumanRunContext>,
        _call: &TaToolCall,
    ) -> bool {
        true
    }

    async fn resolve(
        &self,
        _ctx: &RunContext<OpenHumanRunContext>,
        _call: &TaToolCall,
    ) -> ApprovalResolution {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.answer
            .lock()
            .unwrap()
            .clone()
            .expect("scripted answer")
    }

    fn record(&self, ticket: &str, _result: &ToolResult) {
        self.recorded.lock().unwrap().push(ticket.to_string());
    }
}

struct Tool {
    runs: Arc<AtomicUsize>,
}

impl ToolBaseCall<(), OpenHumanRunContext> for Tool {
    fn call<'a>(
        &'a self,
        _ctx: &'a RunContext<OpenHumanRunContext>,
        _state: &'a (),
        _call: TaToolCall,
    ) -> BoxToolFuture<'a> {
        Box::pin(async move {
            self.runs.fetch_add(1, Ordering::SeqCst);
            Ok(ToolResult::success("ran"))
        })
    }
}

struct Outcome {
    result: ToolResult,
    runs: usize,
    module_asks: usize,
    human_asks: usize,
    recorded: Vec<String>,
}

async fn through_the_gate(
    answer: Result<Verdict, String>,
    human_answer: Option<ApprovalResolution>,
) -> Outcome {
    let module = Arc::new(Module {
        answer,
        asked: AtomicUsize::new(0),
    });
    let human = Arc::new(Human::default());
    *human.answer.lock().unwrap() = human_answer;
    let policy = TinySecurityPolicy::new(module.clone());
    let mut stack: MiddlewareStack<(), OpenHumanRunContext> = MiddlewareStack::new();
    stack.push_tool_middleware(Arc::new(gate(Arc::new(policy), human.clone())));
    let runs = Arc::new(AtomicUsize::new(0));
    let ctx = RunContext::new(RunConfig::new("security-gate"), OpenHumanRunContext::new());
    let call = TaToolCall {
        id: "c1".into(),
        name: "send_email".into(),
        arguments: serde_json::json!({}),
        invalid: None,
    };
    let result = stack
        .run_wrapped_tool(&ctx, &(), call, &Tool { runs: runs.clone() })
        .await
        .unwrap()
        .into_result();
    let recorded = human.recorded.lock().unwrap().clone();
    Outcome {
        result,
        runs: runs.load(Ordering::SeqCst),
        module_asks: module.asked.load(Ordering::SeqCst),
        human_asks: human.asked.load(Ordering::SeqCst),
        recorded,
    }
}

#[tokio::test]
async fn an_allowed_call_runs_without_asking_a_human() {
    let out = through_the_gate(Ok(Verdict::Allow), None).await;
    assert!(!out.result.is_error);
    assert_eq!((out.runs, out.module_asks, out.human_asks), (1, 1, 0));
}

#[tokio::test]
async fn sandbox_required_tools_are_denied_by_the_shared_gate_without_a_sandboxed_workspace() {
    let module = Arc::new(Module {
        answer: Ok(Verdict::Allow),
        asked: AtomicUsize::new(0),
    });
    let policy = TinySecurityPolicy::new(module).with_policy_source(Arc::new(
        super::super::sandbox_requirement::SandboxRequirementPolicy::new(HashMap::from([(
            "send_email".into(),
            tinytools::ToolPolicy::classified().with_runtime(tinytools::ToolRuntime {
                sandbox: tinytools::SandboxMode::Required,
                ..tinytools::ToolRuntime::default()
            }),
        )])),
    ));
    let mut stack: MiddlewareStack<(), OpenHumanRunContext> = MiddlewareStack::new();
    stack.push_tool_middleware(Arc::new(gate(Arc::new(policy), Arc::new(Human::default()))));
    let runs = Arc::new(AtomicUsize::new(0));
    let ctx = RunContext::new(RunConfig::new("security-gate"), OpenHumanRunContext::new());
    let result = stack
        .run_wrapped_tool(
            &ctx,
            &(),
            TaToolCall {
                id: "c1".into(),
                name: "send_email".into(),
                arguments: serde_json::json!({}),
                invalid: None,
            },
            &Tool { runs: runs.clone() },
        )
        .await
        .unwrap()
        .into_result();

    assert!(result.is_error);
    assert!(result.output().contains("requires a sandbox"));
    assert_eq!(runs.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_denied_call_never_runs_and_never_asks_a_human() {
    let out = through_the_gate(
        Ok(Verdict::Deny {
            reason: DenialReason::Floor,
        }),
        Some(ApprovalResolution::Allow { ticket: None }),
    )
    .await;
    assert!(out.result.is_error);
    assert!(out.result.output().contains("tinysecurity"));
    assert_eq!((out.runs, out.human_asks), (0, 0), "deny beats approval");
}

#[tokio::test]
async fn a_call_the_module_leaves_to_a_human_runs_only_when_approved() {
    let needs_human = || {
        Ok(Verdict::Deny {
            reason: DenialReason::ApprovalUnavailable,
        })
    };
    let approved = through_the_gate(
        needs_human(),
        Some(ApprovalResolution::Allow {
            ticket: Some("t-1".into()),
        }),
    )
    .await;
    assert_eq!((approved.runs, approved.human_asks), (1, 1));
    assert_eq!(approved.recorded, vec!["t-1".to_string()]);

    let refused = through_the_gate(
        needs_human(),
        Some(ApprovalResolution::Deny {
            reason: "user said no".into(),
        }),
    )
    .await;
    assert!(refused.result.is_error);
    assert!(refused.result.output().contains("user said no"));
    assert_eq!(refused.runs, 0);
}

#[tokio::test]
async fn a_required_approval_verdict_asks_the_human() {
    let out = through_the_gate(
        Ok(Verdict::RequireApproval {
            request_id: "req-1".into(),
        }),
        Some(ApprovalResolution::Deny {
            reason: "not now".into(),
        }),
    )
    .await;
    assert_eq!((out.runs, out.human_asks), (0, 1));
}

#[tokio::test]
async fn a_module_fault_denies_once_and_does_not_fall_back_to_a_human() {
    let out = through_the_gate(
        Err("module crashed".into()),
        Some(ApprovalResolution::Allow { ticket: None }),
    )
    .await;
    assert!(out.result.is_error);
    assert_eq!(
        (out.runs, out.module_asks, out.human_asks),
        (0, 1, 0),
        "a fault denies with no retry and no approval path"
    );
}

async fn through_combined_gate(
    module: Verdict,
    builder: PolicyDecision,
    human_answer: ApprovalResolution,
) -> (ToolResult, usize, usize) {
    let module = Arc::new(Module {
        answer: Ok(module),
        asked: AtomicUsize::new(0),
    });
    let human = Arc::new(Human::default());
    *human.answer.lock().unwrap() = Some(human_answer);
    let policy = Arc::new(
        TinySecurityPolicy::new(module.clone())
            .with_policy_source(Arc::new(BuilderPolicy(builder))),
    );
    let mut stack: MiddlewareStack<(), OpenHumanRunContext> = MiddlewareStack::new();
    stack.push_tool_middleware(Arc::new(gate(policy, human.clone())));
    let runs = Arc::new(AtomicUsize::new(0));
    let ctx = RunContext::new(
        RunConfig::new("combined-security-gate"),
        OpenHumanRunContext::new(),
    );
    let result = stack
        .run_wrapped_tool(
            &ctx,
            &(),
            TaToolCall {
                id: "c1".into(),
                name: "send_email".into(),
                arguments: serde_json::json!({}),
                invalid: None,
            },
            &Tool { runs: runs.clone() },
        )
        .await
        .unwrap()
        .into_result();
    (
        result,
        runs.load(Ordering::SeqCst),
        human.asked.load(Ordering::SeqCst),
    )
}

#[tokio::test]
async fn builder_approval_uses_the_shared_resolver_after_module_allows() {
    let (result, runs, asks) = through_combined_gate(
        Verdict::Allow,
        PolicyDecision::require_approval("builder approval"),
        ApprovalResolution::Allow {
            ticket: Some("builder-ticket".into()),
        },
    )
    .await;
    assert!(!result.is_error);
    assert_eq!((runs, asks), (1, 1));
}

#[tokio::test]
async fn builder_deny_overrides_module_approval_without_prompting() {
    let (result, runs, asks) = through_combined_gate(
        Verdict::RequireApproval {
            request_id: "module-request".into(),
        },
        PolicyDecision::deny("builder denial"),
        ApprovalResolution::Allow { ticket: None },
    )
    .await;
    assert!(result.is_error);
    assert!(result.output().contains("builder denial"));
    assert_eq!((runs, asks), (0, 0));
}

// The hosted root's own gate used to answer the read-only tier, the hourly budget
// and the scheduled-turn restriction itself. They are the module's answers now, so
// these pin what the model is told when the shared gate relays them, through the
// real derivation of the checks (`ModuleVerdicts`) and a scripted module.

mod relayed_module_answers {
    use super::*;
    use crate::agent::tinyagents::middleware::tinysecurity_module::{ModuleAsk, ModuleVerdicts};
    use tinysecurity_bus::Check;
    use tinytools::{PermissionLevel, Tool as HostTool};

    struct Registered {
        name: &'static str,
        permission: PermissionLevel,
    }

    #[async_trait]
    impl HostTool for Registered {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "test tool"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object" })
        }
        fn permission_level(&self) -> PermissionLevel {
            self.permission
        }
        async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
            unreachable!("the gate never executes a tool")
        }
    }

    /// Answers every question with `answer` and counts how often it was asked.
    struct Scripted {
        answer: Verdict,
        asked: AtomicUsize,
    }

    #[async_trait]
    impl ModuleAsk for Scripted {
        async fn ask(
            &self,
            _ctx: &RunContext<OpenHumanRunContext>,
            _call: &TaToolCall,
            _checks: Vec<Check>,
            _redact: bool,
        ) -> Result<Verdict, String> {
            self.asked.fetch_add(1, Ordering::SeqCst);
            Ok(self.answer.clone())
        }
    }

    async fn relay(
        tool: &'static str,
        permission: PermissionLevel,
        answer: Verdict,
    ) -> (ToolResult, usize, usize) {
        let module = Arc::new(Scripted {
            answer,
            asked: AtomicUsize::new(0),
        });
        let tool_sets = vec![Arc::new(vec![Box::new(Registered {
            name: tool,
            permission,
        }) as Box<dyn HostTool>])];
        let policy = TinySecurityPolicy::new(Arc::new(ModuleVerdicts::with_module(
            tool_sets,
            module.clone(),
        )));
        let mut stack: MiddlewareStack<(), OpenHumanRunContext> = MiddlewareStack::new();
        stack.push_tool_middleware(Arc::new(gate(Arc::new(policy), Arc::new(Human::default()))));
        let runs = Arc::new(AtomicUsize::new(0));
        let ctx = RunContext::new(RunConfig::new("relay"), OpenHumanRunContext::new());
        let call = TaToolCall {
            id: "c1".into(),
            name: tool.into(),
            arguments: serde_json::json!({"path": "a.txt"}),
            invalid: None,
        };
        let result = stack
            .run_wrapped_tool(&ctx, &(), call, &super::Tool { runs: runs.clone() })
            .await
            .unwrap()
            .into_result();
        (
            result,
            runs.load(Ordering::SeqCst),
            module.asked.load(Ordering::SeqCst),
        )
    }

    #[tokio::test]
    async fn the_read_only_tier_denial_keeps_the_policy_marker_on_the_blocked_line() {
        let (result, runs, _) = relay(
            "write_file",
            PermissionLevel::Write,
            Verdict::Deny {
                reason: DenialReason::ReadOnlyMode,
            },
        )
        .await;
        let text = result.output();
        assert!(result.is_error);
        assert_eq!(runs, 0);
        assert!(
            text.contains(tinytools::status::POLICY_BLOCKED_MARKER),
            "the machine-recognisable marker must survive rendering: {text}"
        );
        assert!(text.contains("read-only mode"), "{text}");
        assert!(text.contains("Workaround:"), "{text}");
    }

    #[tokio::test]
    async fn an_exhausted_budget_denies_the_acting_tool_and_names_its_limit() {
        let (result, runs, asked) = relay(
            "write_file",
            PermissionLevel::Write,
            Verdict::Deny {
                reason: DenialReason::RateLimited { max_per_hour: 5 },
            },
        )
        .await;
        assert!(result.is_error);
        assert_eq!((runs, asked), (0, 1));
        let text = result.output();
        assert!(text.contains("Rate limit exceeded"), "{text}");
        assert!(text.contains("5 actions/hour"), "{text}");
    }

    #[tokio::test]
    async fn a_read_runs_while_the_budget_is_spent_and_is_never_asked_about() {
        let (result, runs, asked) = relay(
            "read_file",
            PermissionLevel::ReadOnly,
            Verdict::Deny {
                reason: DenialReason::RateLimited { max_per_hour: 5 },
            },
        )
        .await;
        assert!(!result.is_error, "{}", result.output());
        assert_eq!((runs, asked), (1, 0));
    }

    #[tokio::test]
    async fn a_scheduled_turn_is_told_it_may_use_read_only_tools_only() {
        let (result, runs, _) = relay(
            "write_file",
            PermissionLevel::Write,
            Verdict::Deny {
                reason: DenialReason::ScheduledReadOnly,
            },
        )
        .await;
        assert!(result.is_error);
        assert_eq!(runs, 0);
        assert!(
            result.output().contains("read-only tools only"),
            "{}",
            result.output()
        );
    }

    #[tokio::test]
    async fn an_acting_tool_the_module_allows_runs() {
        let (result, runs, asked) =
            relay("write_file", PermissionLevel::Write, Verdict::Allow).await;
        assert!(!result.is_error);
        assert_eq!((runs, asked), (1, 1));
    }
}

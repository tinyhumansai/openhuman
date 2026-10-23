use super::*;
use std::sync::Mutex;

use serde_json::json;
use tinyagents_harness::context::RunConfig;
use tinysecurity_bus::CommandClass;
use tinytools::ToolResult;

/// Minimal registered tool: this adapter asks about tools, it never runs them.
struct FakeTool {
    name: &'static str,
    permission: PermissionLevel,
    external: bool,
    family: Option<&'static str>,
}

#[async_trait]
impl Tool for FakeTool {
    fn name(&self) -> &str {
        self.name
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
    fn external_effect(&self) -> bool {
        self.external
    }
    fn family(&self) -> Option<&str> {
        self.family
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        unreachable!("the module source never executes a tool")
    }
}

fn registry() -> Vec<Arc<Vec<Box<dyn Tool>>>> {
    let tool = |name, permission, external| -> Box<dyn Tool> {
        Box::new(FakeTool {
            name,
            permission,
            external,
            family: None,
        })
    };
    vec![Arc::new(vec![
        tool("read_file", PermissionLevel::ReadOnly, false),
        tool("memory_meta", PermissionLevel::None, false),
        tool("write_file", PermissionLevel::Write, false),
        tool("run_script", PermissionLevel::Execute, false),
        tool("wipe_disk", PermissionLevel::Dangerous, false),
        tool("gmail_send", PermissionLevel::Write, true),
        tool("web_fetch", PermissionLevel::ReadOnly, true),
        tool("run_workflow", PermissionLevel::None, false),
        tool(SHELL_TOOL, PermissionLevel::Execute, false),
        Box::new(FakeTool {
            name: "desktop_launch",
            permission: PermissionLevel::ReadOnly,
            external: true,
            family: Some("desktop"),
        }),
    ])]
}

type Asked = (Vec<Check>, bool);

/// A module that records what it was asked and answers from a script.
struct Recorder {
    answer: Result<Verdict, String>,
    asked: Mutex<Vec<Asked>>,
}

impl Recorder {
    fn answering(answer: Result<Verdict, String>) -> Arc<Self> {
        Arc::new(Self {
            answer,
            asked: Mutex::new(Vec::new()),
        })
    }
    fn asked(&self) -> Vec<Asked> {
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait]
impl ModuleAsk for Recorder {
    async fn ask(
        &self,
        _ctx: &RunContext<OpenHumanRunContext>,
        _call: &TaToolCall,
        checks: Vec<Check>,
        redact: bool,
    ) -> Result<Verdict, String> {
        self.asked.lock().unwrap().push((checks, redact));
        self.answer.clone()
    }
}

fn ctx() -> RunContext<OpenHumanRunContext> {
    RunContext::new(
        RunConfig::new("module-verdicts"),
        OpenHumanRunContext::new(),
    )
}

fn call(name: &str, arguments: serde_json::Value) -> TaToolCall {
    TaToolCall {
        id: "c1".into(),
        name: name.into(),
        arguments,
        invalid: None,
    }
}

async fn verdict(
    module: &Arc<Recorder>,
    tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
    call: TaToolCall,
) -> Result<Verdict, String> {
    ModuleVerdicts::with_module(tool_sets, module.clone())
        .verdict(&ctx(), &call)
        .await
}

async fn verdict_with_desktop_approvals_disabled(
    module: &Arc<Recorder>,
    call: TaToolCall,
) -> Result<Verdict, String> {
    let mut config = crate::config::Config::default();
    config.desktop.approvals_enabled = false;
    let core = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config,
    );
    let context = RunContext::new(
        RunConfig::new("module-verdicts-desktop-approval-off"),
        OpenHumanRunContext::new().with_core(Some(core)),
    );
    ModuleVerdicts::with_module(registry(), module.clone())
        .verdict(&context, &call)
        .await
}

fn host_action(permission: ToolPermission, external_effect: bool, spend: bool) -> Check {
    Check::HostAction {
        permission,
        external_effect,
        spend,
        command_policy: None,
    }
}

#[test]
fn caller_access_tier_uses_policy_and_never_exceeds_the_dispatch_ceiling() {
    use crate::security::AutonomyLevel::{Full, ReadOnly, Supervised};
    use tinysecurity_bus::AccessTier::{Full as FullAccess, Read, Write};

    assert_eq!(access_tier(ReadOnly, None), Read);
    assert_eq!(access_tier(Supervised, None), Write);
    assert_eq!(access_tier(Full, None), FullAccess);
    assert_eq!(access_tier(Full, Some(ReadOnly)), Read);
    assert_eq!(access_tier(Full, Some(Supervised)), Write);
    assert_eq!(access_tier(ReadOnly, Some(Full)), Read);
}

#[tokio::test]
async fn an_unregistered_tool_is_denied_and_never_asked() {
    let module = Recorder::answering(Ok(Verdict::Allow));
    let answer = verdict(
        &module,
        registry(),
        call("definitely_not_a_tool", json!({})),
    )
    .await;
    assert_eq!(
        answer,
        Ok(Verdict::Deny {
            reason: DenialReason::UnsupportedEffect
        })
    );
    assert!(module.asked().is_empty());
}

#[tokio::test]
async fn an_empty_registry_denies_everything() {
    let module = Recorder::answering(Ok(Verdict::Allow));
    let answer = verdict(&module, Vec::new(), call("read_file", json!({}))).await;
    assert!(matches!(answer, Ok(Verdict::Deny { .. })));
}

#[tokio::test]
async fn a_tool_that_only_reads_is_allowed_without_asking_the_module() {
    let module = Recorder::answering(Err("must not be asked".into()));
    for name in ["read_file", "memory_meta"] {
        let answer = verdict(&module, registry(), call(name, json!({"path": "a"}))).await;
        assert_eq!(answer, Ok(Verdict::Allow), "{name}");
    }
    assert!(module.asked().is_empty());
}

#[tokio::test]
async fn workflow_launch_is_not_skipped_as_a_no_permission_read() {
    let module = Recorder::answering(Ok(Verdict::Allow));
    let answer = verdict(&module, registry(), call("run_workflow", json!({}))).await;

    assert!(
        answer.is_err(),
        "a workflow launch must validate and scan its target"
    );
    assert!(module.asked().is_empty());
}

#[tokio::test]
async fn an_acting_tool_asks_its_permission_level_and_spends_one_action() {
    for (name, permission) in [
        ("write_file", ToolPermission::Write),
        ("run_script", ToolPermission::Execute),
        ("wipe_disk", ToolPermission::Dangerous),
    ] {
        let module = Recorder::answering(Ok(Verdict::Allow));
        let answer = verdict(&module, registry(), call(name, json!({"path": "a"}))).await;
        assert_eq!(answer, Ok(Verdict::Allow), "{name}");
        assert_eq!(
            module.asked(),
            vec![(vec![host_action(permission, false, true)], false)],
            "{name}"
        );
    }
}

#[tokio::test]
async fn a_tool_with_an_external_effect_asks_with_redacted_evidence_and_no_spend() {
    let module = Recorder::answering(Ok(Verdict::Deny {
        reason: DenialReason::ApprovalUnavailable,
    }));
    let answer = verdict(&module, registry(), call("web_fetch", json!({"url": "x"}))).await;
    assert_eq!(
        answer,
        Ok(Verdict::Deny {
            reason: DenialReason::ApprovalUnavailable
        })
    );
    assert_eq!(
        module.asked(),
        vec![(
            vec![host_action(ToolPermission::ReadOnly, true, false)],
            true
        )]
    );
    let module = Recorder::answering(Ok(Verdict::Allow));
    verdict(&module, registry(), call("gmail_send", json!({"to": "x"})))
        .await
        .unwrap();
    assert_eq!(
        module.asked(),
        vec![(vec![host_action(ToolPermission::Write, true, true)], true)]
    );
}

#[tokio::test]
async fn an_unavailable_module_denies_external_effects_without_an_approval_gate() {
    // This process has no interactive ApprovalGate. That must not make a
    // module fault equivalent to permission for an external effect.
    let module = Recorder::answering(Err("module unavailable".into()));
    let answer = verdict(&module, registry(), call("web_fetch", json!({"url": "x"}))).await;

    assert_eq!(answer, Err("module unavailable".into()));
    assert_eq!(
        module.asked(),
        vec![(
            vec![host_action(ToolPermission::ReadOnly, true, false)],
            true
        )]
    );
}

#[tokio::test]
async fn desktop_approval_bypass_keeps_external_metadata_and_fails_closed_on_module_fault() {
    let module = Recorder::answering(Err("module unavailable".into()));
    let answer = verdict_with_desktop_approvals_disabled(
        &module,
        call("desktop_launch", json!({"app": "Calculator"})),
    )
    .await;

    assert_eq!(answer, Err("module unavailable".into()));
    assert_eq!(
        module.asked(),
        vec![(
            vec![host_action(ToolPermission::ReadOnly, true, false)],
            true
        )]
    );
}

#[tokio::test]
async fn shell_asks_for_its_command_without_spending_and_the_category_only_raises() {
    for (category, expected) in [
        (Some("destructive"), Some(CommandClass::Destructive)),
        (Some("READ"), Some(CommandClass::Read)),
        (Some("bogus"), None),
        (None, None),
    ] {
        let module = Recorder::answering(Ok(Verdict::Allow));
        let mut arguments = json!({"command": "ls -la"});
        if let Some(category) = category {
            arguments["category"] = json!(category);
        }
        let answer = verdict(&module, registry(), call(SHELL_TOOL, arguments)).await;
        assert_eq!(answer, Ok(Verdict::Allow));
        assert_eq!(
            module.asked(),
            vec![(
                vec![Check::HostShell {
                    command: "ls -la".into(),
                    declared_class: expected,
                    spend: false,
                    command_policy: None,
                }],
                false
            )],
            "{category:?}"
        );
    }
}

#[tokio::test]
async fn a_shell_call_without_a_command_is_denied_and_never_asked() {
    let module = Recorder::answering(Ok(Verdict::Allow));
    for arguments in [json!({}), json!({"command": 3})] {
        let answer = verdict(&module, registry(), call(SHELL_TOOL, arguments)).await;
        assert!(matches!(answer, Ok(Verdict::Deny { .. })));
    }
    assert!(module.asked().is_empty());
}

#[tokio::test]
async fn the_module_verdict_passes_through_unchanged() {
    for answer in [
        Verdict::Deny {
            reason: DenialReason::RateLimited { max_per_hour: 9 },
        },
        Verdict::Deny {
            reason: DenialReason::ReadOnlyMode,
        },
        Verdict::RequireApproval {
            request_id: "r".into(),
        },
    ] {
        let module = Recorder::answering(Ok(answer.clone()));
        assert_eq!(
            verdict(&module, registry(), call("write_file", json!({}))).await,
            Ok(answer)
        );
    }
}

#[tokio::test]
async fn with_no_approval_gate_a_module_fault_still_blocks_acting_tools() {
    // No interactive approval UI is installed in this process, but the module
    // still owns admission for acting tools.
    assert!(ApprovalGate::try_global().is_none());
    let module = Recorder::answering(Err("module unavailable".into()));
    assert_eq!(
        verdict(&module, registry(), call("write_file", json!({}))).await,
        Err("module unavailable".to_string())
    );
    assert_eq!(
        verdict(
            &module,
            registry(),
            call(SHELL_TOOL, json!({"command": "ls"}))
        )
        .await,
        Err("module unavailable".to_string())
    );
}

#[test]
fn the_cron_origin_reaches_the_module_and_other_origins_stay_distinct() {
    let mut cron = ctx();
    cron.data.origin = Some(AgentTurnOrigin::TrustedAutomation {
        job_id: "cron-1".into(),
        source: TrustedAutomationSource::Cron,
    });
    assert_eq!(module_origin(&cron), Origin::Cron);

    assert_eq!(module_origin(&ctx()), Origin::Operator);
    let mut copilot = ctx();
    copilot.data.approval.copilot_stream = true;
    assert_eq!(module_origin(&copilot), Origin::Copilot);
    let mut child = ctx();
    child.data.spawn_depth = 1;
    assert_eq!(module_origin(&child), Origin::SubAgent);
}

#[tokio::test]
async fn a_scheduled_origin_in_the_task_scope_reaches_the_module_too() {
    let origin = AgentTurnOrigin::TrustedAutomation {
        job_id: "cron-1".into(),
        source: TrustedAutomationSource::Cron,
    };
    let seen =
        crate::security::turn_origin::with_origin(origin, async { module_origin(&ctx()) }).await;
    assert_eq!(seen, Origin::Cron);
}

#[test]
fn permission_levels_map_one_to_one() {
    for (level, expected) in [
        (PermissionLevel::None, ToolPermission::None),
        (PermissionLevel::ReadOnly, ToolPermission::ReadOnly),
        (PermissionLevel::Write, ToolPermission::Write),
        (PermissionLevel::Execute, ToolPermission::Execute),
        (PermissionLevel::Dangerous, ToolPermission::Dangerous),
    ] {
        assert_eq!(permission(level), expected);
        assert_eq!(level_acts(level), expected.acts());
    }
}

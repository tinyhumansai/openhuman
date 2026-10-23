//! The production [`VerdictSource`]: ask the TinySecurity module whether a tool
//! call may run (issue #7328, phase 6).
//!
//! Every tool that could be refused reaches the module as a host-tool admission
//! question (`Check::HostAction`, or `Check::HostShell` for `shell`); the module
//! decides from the verified caller (origin, tier), the tool's declared permission
//! level and external effect, and the command scope registered for the user's
//! autonomy settings (its tier and hourly budget). This file derives the checks
//! from the tool the host will execute, never from model arguments, and passes the
//! module's answer through unchanged; mapping it onto the harness decision and
//! the model-facing text is [`super::tinysecurity`]'s job.
//!
//! The one answer the host does not ask for is the one the module could only give
//! one way: a tool that neither acts nor has an external effect (and is not
//! `shell`) is allowed in every tier and origin, so it runs without a bus call.
//!
//! Any failure to obtain an answer is an `Err`, which the policy turns into a
//! denial. The one exception is a process that never installed the approval gate
//! (headless embed, CLI, tests): it has no human to ask, the resolver has always
//! let such a call through, and so a module that cannot answer does not stop a
//! non-shell tool there. `shell` is always refused without an answer.

use std::sync::Arc;

use async_trait::async_trait;
use tinyagents_harness::context::RunContext;
use tinyinference_llm::tool::ToolCall as TaToolCall;
#[cfg(feature = "skills")]
use tinysecurity_bus::SkillScanVerdict;
use tinysecurity_bus::{
    AccessTier, CallerContext, Check, DenialReason, Origin, ToolCall, ToolPermission, Verdict,
};
use tinytools::{PermissionLevel, Tool};

use super::approval::{approval_tool_name, ApprovalSecurityMiddleware};
use super::tinysecurity::{VerdictSource, SHELL_TOOL};
use crate::agent::tinyagents::host::OpenHumanRunContext;
#[cfg(test)]
use crate::security::approval::ApprovalGate;
use crate::security::policy::{AutonomyLevel, CommandFlow, CommandStage, SecurityPolicy};
use crate::security::turn_origin::{AgentTurnOrigin, TrustedAutomationSource};

const LOG: &str = "[tinyagents::mw::tinysecurity]";

/// Puts host-derived checks to the module and returns its verdict. Production is
/// [`BusModule`]; the seam lets the derivation below be tested without a loaded
/// module. `Err` is a fault: the policy denies and never retries.
#[async_trait]
pub(crate) trait ModuleAsk: Send + Sync {
    async fn ask(
        &self,
        ctx: &RunContext<OpenHumanRunContext>,
        call: &TaToolCall,
        checks: Vec<Check>,
        redact: bool,
    ) -> Result<Verdict, String>;
}

/// Asks the module through `modules::security::evaluate`.
pub(crate) struct ModuleVerdicts {
    /// Resolves a call's tool and its external-effect declaration, and applies
    /// the desktop approvals-off exception the approval resolver always applied.
    effects: ApprovalSecurityMiddleware,
    module: Arc<dyn ModuleAsk>,
}

impl ModuleVerdicts {
    pub(crate) fn new(tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>) -> Self {
        Self::with_module(tool_sets, Arc::new(BusModule))
    }

    pub(crate) fn with_module(
        tool_sets: Vec<Arc<Vec<Box<dyn Tool>>>>,
        module: Arc<dyn ModuleAsk>,
    ) -> Self {
        Self {
            effects: ApprovalSecurityMiddleware::new(tool_sets),
            module,
        }
    }
}

/// The origin of the turn the call belongs to: the run's own, else the one the
/// dispatch or the calling task's turn scope carries.
fn turn_origin(ctx: &RunContext<OpenHumanRunContext>) -> Option<AgentTurnOrigin> {
    ctx.data
        .origin
        .clone()
        .or_else(|| ctx.data.dispatch_ctx().turn_origin_or_scoped())
}

/// The origin class the module sees, from the verified turn context.
pub(super) fn module_origin(ctx: &RunContext<OpenHumanRunContext>) -> Origin {
    if matches!(
        turn_origin(ctx),
        Some(AgentTurnOrigin::TrustedAutomation {
            source: TrustedAutomationSource::Cron,
            ..
        })
    ) {
        Origin::Cron
    } else if crate::core::runtime::is_saas() {
        Origin::Saas
    } else if ctx.data.approval.copilot_stream {
        Origin::Copilot
    } else if ctx.data.spawn_depth > 0 {
        Origin::SubAgent
    } else {
        Origin::Operator
    }
}

/// The module's permission vocabulary for the host's tool permission level.
fn permission(level: PermissionLevel) -> ToolPermission {
    match level {
        PermissionLevel::None => ToolPermission::None,
        PermissionLevel::ReadOnly => ToolPermission::ReadOnly,
        PermissionLevel::Write => ToolPermission::Write,
        PermissionLevel::Execute => ToolPermission::Execute,
        PermissionLevel::Dangerous => ToolPermission::Dangerous,
    }
}

#[async_trait]
impl VerdictSource for ModuleVerdicts {
    async fn verdict(
        &self,
        ctx: &RunContext<OpenHumanRunContext>,
        call: &TaToolCall,
    ) -> Result<Verdict, String> {
        let shell = call.name == SHELL_TOOL;
        #[cfg(feature = "skills")]
        let workflow_run = call.name == crate::skills::run_workflow::RUN_WORKFLOW_TOOL_NAME;
        #[cfg(not(feature = "skills"))]
        let workflow_run = false;
        // Unknown name: nothing to reason about, and "unknown" never means fine.
        let Some((resolved_level, declared_external)) =
            self.effects.resolve_tool(&call.name).map(|tool| {
                (
                    tool.permission_level_with_args(&call.arguments),
                    tool.external_effect_with_args(&call.arguments),
                )
            })
        else {
            tracing::warn!(tool = %call.name, "{LOG} denied: tool is not registered in this session");
            return Ok(Verdict::Deny {
                reason: DenialReason::UnsupportedEffect,
            });
        };
        // A workflow launches a separate autonomous agent. Its tool declaration
        // is intentionally `None` because its target policy differs; admission
        // still needs a host-effect check paired with the exact bundle scan.
        let level = if workflow_run {
            PermissionLevel::Execute
        } else {
            resolved_level
        };
        // Approval posture may waive a human prompt, but it must not rewrite
        // the tool's declared external effect. TinySecurity needs the real
        // effect metadata to enforce privacy and policy in every posture.
        let external = declared_external || workflow_run;
        let acts = level_acts(level);
        let (checks, redact) = if shell {
            let Some(check) = shell_check(&call.arguments) else {
                return Ok(Verdict::Deny {
                    reason: DenialReason::Command,
                });
            };
            (vec![check], false)
        } else if !acts && !external {
            // The module allows a reading tool in every tier and origin.
            return Ok(Verdict::Allow);
        } else {
            let check = Check::HostAction {
                permission: permission(level),
                external_effect: external,
                // An allowed acting call spends one action of the hourly budget.
                spend: acts && !workflow_run,
                command_policy: None,
            };
            #[cfg(feature = "skills")]
            let mut checks = vec![check];
            #[cfg(not(feature = "skills"))]
            let checks = vec![check];
            #[cfg(feature = "skills")]
            if workflow_run {
                checks.push(workflow_skill_scan_check(ctx, call).await?);
            }
            (checks, external)
        };
        self.module.ask(ctx, call, checks, redact).await
    }
}

#[cfg(feature = "skills")]
async fn workflow_skill_scan_check(
    ctx: &RunContext<OpenHumanRunContext>,
    call: &TaToolCall,
) -> Result<Check, String> {
    let workflow_id = call
        .arguments
        .get("workflow_id")
        .or_else(|| call.arguments.get("skill_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "run_workflow is missing a workflow id".to_string())?;
    let workspace = crate::skills::schemas::resolve_workspace_dir().await;
    let (workflow, source) =
        crate::skills::registry::get_workflow_with_source(&workspace, workflow_id)
            .ok_or_else(|| format!("run_workflow references unknown workflow '{workflow_id}'"))?;
    let scan = crate::skills::runtime::skill_scan::scan_workflow(&workflow, source.as_ref())?;
    ctx.data.record_workflow_scan_permit(
        workflow_id,
        call.arguments
            .get("inputs")
            .unwrap_or(&serde_json::Value::Null),
        &call.id,
        &scan.bundle_sha256,
    );
    Ok(Check::SkillScan {
        verdict: match scan.verdict {
            tinyskills::Verdict::Pass => SkillScanVerdict::Pass,
            tinyskills::Verdict::Warn => SkillScanVerdict::Warn,
            tinyskills::Verdict::Block => SkillScanVerdict::Block,
        },
        bundle_sha256: scan.bundle_sha256,
    })
}

fn level_acts(level: PermissionLevel) -> bool {
    level > PermissionLevel::ReadOnly
}

/// The module receives the effective host tier: the configured autonomy
/// level narrowed by any caller-specific ceiling carried by this dispatch.
fn access_tier(autonomy: AutonomyLevel, ceiling: Option<AutonomyLevel>) -> AccessTier {
    if autonomy == AutonomyLevel::ReadOnly || ceiling == Some(AutonomyLevel::ReadOnly) {
        AccessTier::Read
    } else if autonomy == AutonomyLevel::Supervised || ceiling == Some(AutonomyLevel::Supervised) {
        AccessTier::Write
    } else {
        AccessTier::Full
    }
}

/// The shell command gate for `arguments`, or `None` when the call carries no
/// command to classify. The model's `category` can only raise the class.
fn shell_check(arguments: &serde_json::Value) -> Option<Check> {
    let command = arguments
        .get("command")
        .and_then(serde_json::Value::as_str)?;
    let declared_class = arguments
        .get("category")
        .and_then(serde_json::Value::as_str)
        .and_then(SecurityPolicy::parse_declared_class);
    Some(Check::HostShell {
        command: command.to_owned(),
        declared_class,
        // The shell tool reserves an action when it runs; asking never does.
        spend: false,
        command_policy: None,
    })
}

/// The production module client: the live bus.
pub(crate) struct BusModule;

#[async_trait]
impl ModuleAsk for BusModule {
    async fn ask(
        &self,
        ctx: &RunContext<OpenHumanRunContext>,
        call: &TaToolCall,
        mut checks: Vec<Check>,
        redact: bool,
    ) -> Result<Verdict, String> {
        let dispatch = &ctx.data.dispatch_ctx();
        let config = dispatch.load_config().await?;
        // The scope the shell tool spends its hourly budget through: the same
        // settings, so an equal policy reuses the registered scope and its ledger.
        let policy = crate::security::live_policy::effective_in(dispatch)
            .unwrap_or_else(|| Arc::new(SecurityPolicy::from_full_config(&config)));
        let tier = access_tier(policy.autonomy, dispatch.autonomy_tier_ceiling());
        let scope = crate::modules::security::command::command_scope_id(
            &policy.workspace_dir,
            policy
                .native_command_policy(CommandFlow::Harness, CommandStage::Execute)
                .map_err(|refusal| refusal.to_string())?,
        )
        .await?;
        for check in &mut checks {
            match check {
                Check::HostAction { command_policy, .. }
                | Check::HostShell { command_policy, .. } => *command_policy = Some(scope.clone()),
                _ => {}
            }
        }
        let tenant = dispatch
            .tenant()
            .map_err(|_| "no authenticated tenant scope".to_string())?;
        let agent = tenant
            .agent
            .filter(|agent| !agent.trim().is_empty())
            .unwrap_or_else(|| "orchestrator".to_string());
        let call_id = if call.id.trim().is_empty() {
            format!("tool-{}", call.name)
        } else {
            call.id.clone()
        };
        let caller = crate::modules::security::VerifiedCaller::from_host(CallerContext {
            agent,
            thread: ctx.config.thread_id.as_ref().map(ToString::to_string),
            flow: ctx
                .data
                .approval
                .flow_run
                .as_ref()
                .map(|run| run.flow_id.clone()),
            call_id,
            origin: module_origin(ctx),
            tier,
        })?;
        let tool = approval_tool_name(&call.name, &call.arguments).into_owned();
        // Arguments are only evidence for a human's approval; a call that needs
        // none is judged by its checks alone.
        let arguments_json = if redact {
            serde_json::to_string(&crate::security::redact::arguments(&call.arguments))
                .map_err(|_| "argument encoding failed".to_string())?
        } else {
            "{}".to_string()
        };
        let decision = crate::modules::security::evaluate(
            &config,
            caller,
            ToolCall {
                tool,
                arguments_json,
                checks,
            },
        )
        .await?;
        Ok(decision.verdict)
    }
}

#[cfg(test)]
#[path = "tinysecurity_module_tests.rs"]
mod tests;

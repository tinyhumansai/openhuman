//! OpenHuman policy adapters for TinyAgents' generic execution tools.

use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use serde_json::Value;
use tinyagents_harness_tools::exec::{
    ExecAuditEvent, ExecAuditOutcome, ExecAuthorizationRequest, ExecBackendCapability, ExecDenied,
    ExecGate, ExecOperation, ExecPermit, ExecRefusalKind, NativeExecCapability,
};
use tinytools::ToolRunContext;

use crate::{
    agent::tinyagents::host::run_context::approval_for_tool,
    config::Config,
    core::runtime::DispatchCtx,
    security::{audit_record::HostAuditEvent, AuditSink, SecurityPolicy},
};

/// Host-owned gate for package installation. The generic TinyAgents tool owns
/// argument validation and process-result formatting; OpenHuman owns whether
/// a host install is permitted.
pub(super) struct InstallGate {
    pub security: Arc<SecurityPolicy>,
    pub audit: Arc<AuditSink>,
    pub config: Arc<Config>,
    pub(super) audit_context: DispatchCtx,
    pub environment: std::collections::BTreeMap<std::ffi::OsString, std::ffi::OsString>,
}

impl InstallGate {
    /// Make an invocation-owned gate so each `ExecGate::record` retains the
    /// exact dispatch that authorized that execution, even when tool calls
    /// from one turn run concurrently.
    pub(super) fn for_tool_context(&self, context: Option<&dyn ToolRunContext>) -> Self {
        Self {
            security: Arc::clone(&self.security),
            audit: Arc::clone(&self.audit),
            config: Arc::clone(&self.config),
            audit_context: crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(
                context,
            ),
            environment: self.environment.clone(),
        }
    }
}

#[async_trait]
impl ExecGate for InstallGate {
    async fn prepare_authorization(&self, _args: &Value) -> anyhow::Result<Option<Value>> {
        Ok(None)
    }

    fn external_effect_with_args(&self, _args: &Value) -> bool {
        true
    }

    async fn authorize(
        &self,
        request: &ExecAuthorizationRequest,
        context: Option<&dyn ToolRunContext>,
    ) -> Result<ExecPermit, ExecDenied> {
        let ExecOperation::Install { .. } = &request.operation else {
            return Err(ExecDenied(
                "install gate received a non-install operation".into(),
            ));
        };
        if crate::core::runtime::is_saas() {
            return Err(ExecDenied(
                "[policy-denied] Package installation is unavailable in SaaS mode.".into(),
            ));
        }
        if !approval_for_tool(context).is_interactive() {
            return Err(ExecDenied(
                "[policy-denied] install_tool requires interactive approval and is not available in autonomous (background) turns.".into(),
            ));
        }
        if !self.security.allow_tool_install {
            return Err(ExecDenied(
                "[policy-denied] OS package installation is disabled. Enable it in the agent access settings (Full access mode / allow_tool_install) before installing tools.".into(),
            ));
        }
        let ExecOperation::Install { package, manager } = &request.operation else {
            unreachable!("operation checked above")
        };
        let command = format!("{manager} install {package}");
        match self
            .security
            .check_command(crate::security::policy::CommandQuery {
                command: &command,
                declared: Some(crate::security::policy::CommandClass::Install),
                flow: crate::security::policy::CommandFlow::Harness,
                stage: crate::security::policy::CommandStage::Execute,
            })
            .await
        {
            Ok(verdict) if verdict.gate != crate::security::policy::GateDecision::Block => {}
            Ok(_) => {
                return Err(ExecDenied(
                    "[policy-blocked] Install action denied by TinySecurity.".into(),
                ))
            }
            Err(reason) => return Err(ExecDenied(reason.to_string())),
        }
        Ok(ExecPermit {
            action_dir: PathBuf::from(&self.security.workspace_dir),
            environment: self.environment.clone(),
            backend_selection: "native".into(),
            backend_capability: ExecBackendCapability::new(NativeExecCapability),
        })
    }

    fn record(&self, event: ExecAuditEvent) {
        let command = match &event.operation {
            ExecOperation::Install { package, manager } => format!("{manager} install {package}"),
            ExecOperation::Shell { command, .. } => command.clone(),
        };
        let success = matches!(
            event.outcome,
            ExecAuditOutcome::Completed { success: true, .. }
        );
        if self.audit.is_enabled() {
            let ctx = self.audit_context.clone();
            let config = Arc::clone(&self.config);
            let event = HostAuditEvent::command_execution(
                "tool:install_tool",
                &command,
                event.approval_required,
                event.allowed,
                success,
                u64::try_from(event.duration.as_millis()).unwrap_or(u64::MAX),
            );
            tokio::spawn(async move {
                if let Err(error) =
                    crate::security::audit_record::record(&ctx, Some(&config), event).await
                {
                    tracing::warn!(error = %error, "[install_tool] execution audit row was not recorded");
                }
            });
        }
        if let ExecAuditOutcome::BackendRefused {
            kind: ExecRefusalKind::BackendUnavailable,
        } = event.outcome
        {
            tracing::warn!("[install_tool] selected execution backend is unavailable");
        }
    }
}

#[cfg(test)]
#[path = "exec_adapters_tests.rs"]
mod tests;

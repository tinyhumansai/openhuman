//! Tool: install_tool — install an OS / language package, heavily gated.
//!
//! Host package-manager execution is disabled until the installer can run inside
//! an approved isolated environment. Human approval does not isolate package
//! install scripts from the host.

use crate::security::SecurityPolicy;
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tinytools::{PermissionLevel, Tool, ToolResult};

pub struct InstallToolTool {
    security: Arc<SecurityPolicy>,
}

impl InstallToolTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }
}

/// The refusal returned to a SaaS tenant, or `None` for a single-user host.
fn refuse_under_saas(saas: bool) -> Option<ToolResult> {
    saas.then(|| {
        tracing::warn!("[install_tool] refused under SaaS");
        ToolResult::error(
            "[policy-denied] install_tool is not available in SaaS mode: it would change the \
             shared host, not the tenant's sandbox.",
        )
    })
}

#[async_trait]
impl Tool for InstallToolTool {
    fn name(&self) -> &str {
        "install_tool"
    }

    fn description(&self) -> &str {
        "OS package installation is currently unavailable because package-manager scripts are not \
         confined to an approved isolated environment."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "package": {
                    "type": "string",
                    "description": "Package name to install (e.g. 'ripgrep', 'jq', '@scope/cli')."
                },
                "manager": {
                    "type": "string",
                    "enum": ["apt", "apt-get", "dnf", "yum", "pacman", "apk", "brew", "winget", "pipx", "npm", "cargo"],
                    "description": "Package manager to use. If omitted, the host's system package manager is auto-detected."
                }
            },
            "required": ["package"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Dangerous
    }

    fn external_effect(&self) -> bool {
        true
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
        let approval = crate::agent::tinyagents::host::run_context::approval_for_tool(context);
        self.run(args, &approval).await
    }
}

impl InstallToolTool {
    /// Runs the install for a turn whose approval context is `approval`.
    pub(crate) async fn run(
        &self,
        _args: serde_json::Value,
        approval: &crate::security::approval::ApprovalTurnContext,
    ) -> anyhow::Result<ToolResult> {
        // Gate -1: a package install changes the machine the core runs on, which
        // under SaaS is shared by every tenant. Never, for any tenant.
        if let Some(refusal) = refuse_under_saas(crate::core::runtime::is_saas()) {
            return Ok(refusal);
        }
        // Gate 0: installs mutate the host (not the workspace), so they must
        // never run without an explicit human Approve. The ApprovalGate only
        // parks for an interactive turn (a chat context on the turn's approval
        // context, set on the web-chat path); background / triage / cron turns
        // bypass the gate entirely. Fail closed there — a doomed retry is
        // short-circuited by the `[policy-denied]` marker.
        if !approval.is_interactive() {
            return Ok(ToolResult::error(
                "[policy-denied] install_tool requires interactive approval and is not available \
                 in autonomous (background) turns.",
            ));
        }
        // Gate 1: feature must be explicitly enabled (Full access mode).
        if !self.security.allow_tool_install {
            return Ok(ToolResult::error(
                "[policy-denied] OS package installation is disabled. Enable it in the agent access settings \
                 (Full access mode / allow_tool_install) before installing tools.",
            ));
        }
        Ok(ToolResult::error(
            "OS package installation is unavailable until an isolated package installer exists.",
        ))
    }
}

#[cfg(test)]
#[path = "install_tool_tests.rs"]
mod tests;

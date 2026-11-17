use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinyagents_harness_tools::exec::{ExecBackend, InstallTool};
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult, ToolRunContext, ToolTimeout};

use crate::config::Config;
use crate::security::{AuditSink, SecurityPolicy};

use super::exec_adapters::InstallGate;

/// OpenHuman host adapter for TinyAgents' generic package installer.
pub struct InstallToolTool {
    inner: InstallTool,
    gate: Arc<InstallGate>,
    backend: Arc<dyn ExecBackend>,
}

impl InstallToolTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self::with_audit(security, AuditSink::disabled())
    }

    pub fn with_audit(security: Arc<SecurityPolicy>, audit: Arc<AuditSink>) -> Self {
        Self::with_audit_and_config(security, audit, Arc::new(Config::default()))
    }

    pub fn with_audit_and_config(
        security: Arc<SecurityPolicy>,
        audit: Arc<AuditSink>,
        config: Arc<Config>,
    ) -> Self {
        Self::with_backend_and_environment(
            security,
            audit,
            config,
            Arc::new(tinyagents_harness_tools::exec::NativeExecBackend::new(
                tinyagents_harness_tools::exec::ShellFlavor::Posix,
            )),
            install_environment(),
        )
    }

    fn with_backend_and_environment(
        security: Arc<SecurityPolicy>,
        audit: Arc<AuditSink>,
        config: Arc<Config>,
        backend: Arc<dyn ExecBackend>,
        environment: std::collections::BTreeMap<std::ffi::OsString, std::ffi::OsString>,
    ) -> Self {
        let gate = Arc::new(InstallGate {
            security,
            audit,
            config,
            audit_context: crate::core::runtime::DispatchCtx::unbound(),
            environment,
        });
        let inner = InstallTool::new(gate.clone(), Arc::clone(&backend));
        Self {
            inner,
            gate,
            backend,
        }
    }

    fn invocation(&self, context: Option<&dyn ToolRunContext>) -> InstallTool {
        InstallTool::new(
            Arc::new(self.gate.for_tool_context(context)),
            Arc::clone(&self.backend),
        )
    }
}

fn install_environment() -> std::collections::BTreeMap<std::ffi::OsString, std::ffi::OsString> {
    filter_install_environment(std::env::vars_os())
}

fn filter_install_environment(
    variables: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> std::collections::BTreeMap<std::ffi::OsString, std::ffi::OsString> {
    const ALLOWED: &[&str] = &[
        "PATH",
        "HOME",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "SYSTEMROOT",
        "WINDIR",
        "PATHEXT",
        "TEMP",
        "TMP",
        "TMPDIR",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "PIPX_HOME",
        "PIPX_BIN_DIR",
        "NPM_CONFIG_PREFIX",
        "HOMEBREW_PREFIX",
        "HOMEBREW_CELLAR",
        "HOMEBREW_REPOSITORY",
        "XDG_CACHE_HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
    ];
    variables
        .into_iter()
        .filter(|(name, _)| {
            name.to_str().is_some_and(|name| {
                ALLOWED
                    .iter()
                    .any(|allowed| name.eq_ignore_ascii_case(allowed))
            })
        })
        .collect()
}

#[async_trait]
impl Tool for InstallToolTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters_schema(&self) -> Value {
        self.inner.parameters_schema()
    }

    fn permission_level(&self) -> PermissionLevel {
        self.inner.permission_level()
    }

    fn external_effect(&self) -> bool {
        self.inner.external_effect()
    }

    fn external_effect_with_args(&self, args: &Value) -> bool {
        self.inner.external_effect_with_args(args)
    }

    fn timeout_policy(&self, args: &Value) -> ToolTimeout {
        self.inner.timeout_policy(args)
    }

    async fn prepare_authorization(&self, args: &Value) -> anyhow::Result<Option<Value>> {
        self.inner.prepare_authorization(args).await
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.invocation(None).execute(args).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        self.invocation(context)
            .execute_with_context(args, options, context)
            .await
    }
}

#[cfg(test)]
#[path = "install_tool_tests.rs"]
mod tests;

//! [`CeilingGuard`]'s `Tool` impl: everything forwards except execution,
//! which is refused above the ceiling.

use serde_json::Value;
use std::any::Any;
use tinytools::{
    PermissionLevel, Tool, ToolCallOptions, ToolCategory, ToolExposure, ToolInjectedArgument,
    ToolPolicy, ToolResult, ToolRunContext, ToolScope, ToolSpec, ToolTimeout,
};

use super::CeilingGuard;

#[async_trait::async_trait]
impl Tool for CeilingGuard {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn description(&self) -> &str {
        self.inner.description()
    }
    fn parameters_schema(&self) -> Value {
        self.inner.parameters_schema()
    }
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        if let Some(refused) = self.refusal(&args) {
            return Ok(refused);
        }
        self.inner.execute(args).await
    }
    async fn execute_with_options(
        &self,
        args: Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        if let Some(refused) = self.refusal(&args) {
            return Ok(refused);
        }
        self.inner.execute_with_options(args, options).await
    }
    async fn execute_with_context(
        &self,
        args: Value,
        options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        if let Some(refused) = self.refusal(&args) {
            return Ok(refused);
        }
        self.inner
            .execute_with_context(args, options, context)
            .await
    }
    fn policy(&self) -> ToolPolicy {
        self.inner.policy()
    }
    fn injected_arguments(&self) -> Vec<ToolInjectedArgument> {
        self.inner.injected_arguments()
    }
    fn supports_markdown(&self) -> bool {
        self.inner.supports_markdown()
    }
    fn permission_level(&self) -> PermissionLevel {
        self.inner.permission_level()
    }
    fn permission_level_with_args(&self, args: &Value) -> PermissionLevel {
        self.inner.permission_level_with_args(args)
    }
    fn scope(&self) -> ToolScope {
        self.inner.scope()
    }
    fn category(&self) -> ToolCategory {
        self.inner.category()
    }
    fn exposure(&self) -> ToolExposure {
        self.inner.exposure()
    }
    fn family(&self) -> Option<&str> {
        self.inner.family()
    }
    fn is_concurrency_safe(&self, args: &Value) -> bool {
        self.inner.is_concurrency_safe(args)
    }
    // The guard answers an external-effect call itself (by refusing it), so
    // it reports none: the approval gate must not park a call that will be
    // refused anyway.
    fn external_effect(&self) -> bool {
        false
    }
    fn external_effect_with_args(&self, _args: &Value) -> bool {
        false
    }
    fn max_result_size_chars(&self) -> Option<usize> {
        self.inner.max_result_size_chars()
    }
    fn timeout_policy(&self, args: &Value) -> ToolTimeout {
        self.inner.timeout_policy(args)
    }
    fn host_extension(&self) -> Option<&(dyn Any + Send + Sync)> {
        self.inner.host_extension()
    }
    fn host_call_extension(&self, args: &Value) -> Option<Box<dyn Any + Send + Sync>> {
        self.inner.host_call_extension(args)
    }
    fn spec(&self) -> ToolSpec {
        self.inner.spec()
    }
    fn display_label(&self, args: &Value) -> Option<String> {
        self.inner.display_label(args)
    }
    fn display_detail(&self, args: &Value) -> Option<String> {
        self.inner.display_detail(args)
    }
    fn return_direct(&self) -> bool {
        self.inner.return_direct()
    }
}

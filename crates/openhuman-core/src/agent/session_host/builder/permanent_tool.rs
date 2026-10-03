//! Preserve a host tool's execution metadata while fixing its direct exposure.
use serde_json::Value;
use std::any::Any;
use tinytools::{
    PermissionLevel, Tool, ToolCallOptions, ToolCategory, ToolExposure, ToolInjectedArgument,
    ToolPolicy, ToolResult, ToolRunContext, ToolScope, ToolSpec, ToolTimeout,
};
pub(super) struct PermanentTool(pub Box<dyn Tool>);
#[async_trait::async_trait]
impl Tool for PermanentTool {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn description(&self) -> &str {
        self.0.description()
    }
    fn parameters_schema(&self) -> Value {
        self.0.parameters_schema()
    }
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Direct
    }
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.0.execute(args).await
    }
    async fn execute_with_options(
        &self,
        args: Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        self.0.execute_with_options(args, options).await
    }
    async fn execute_with_context(
        &self,
        args: Value,
        options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        self.0.execute_with_context(args, options, context).await
    }
    fn policy(&self) -> ToolPolicy {
        self.0.policy()
    }
    fn injected_arguments(&self) -> Vec<ToolInjectedArgument> {
        self.0.injected_arguments()
    }
    fn supports_markdown(&self) -> bool {
        self.0.supports_markdown()
    }
    fn permission_level(&self) -> PermissionLevel {
        self.0.permission_level()
    }
    fn permission_level_with_args(&self, args: &Value) -> PermissionLevel {
        self.0.permission_level_with_args(args)
    }
    fn scope(&self) -> ToolScope {
        self.0.scope()
    }
    fn category(&self) -> ToolCategory {
        self.0.category()
    }
    fn family(&self) -> Option<&str> {
        self.0.family()
    }
    fn is_concurrency_safe(&self, args: &Value) -> bool {
        self.0.is_concurrency_safe(args)
    }
    fn external_effect(&self) -> bool {
        self.0.external_effect()
    }
    fn external_effect_with_args(&self, args: &Value) -> bool {
        self.0.external_effect_with_args(args)
    }
    fn max_result_size_chars(&self) -> Option<usize> {
        self.0.max_result_size_chars()
    }
    fn timeout_policy(&self, args: &Value) -> ToolTimeout {
        self.0.timeout_policy(args)
    }
    fn host_extension(&self) -> Option<&(dyn Any + Send + Sync)> {
        self.0.host_extension()
    }
    fn host_call_extension(&self, args: &Value) -> Option<Box<dyn Any + Send + Sync>> {
        self.0.host_call_extension(args)
    }
    fn spec(&self) -> ToolSpec {
        self.0.spec()
    }
    fn display_label(&self, args: &Value) -> Option<String> {
        self.0.display_label(args)
    }
    fn display_detail(&self, args: &Value) -> Option<String> {
        self.0.display_detail(args)
    }
    fn return_direct(&self) -> bool {
        self.0.return_direct()
    }
}

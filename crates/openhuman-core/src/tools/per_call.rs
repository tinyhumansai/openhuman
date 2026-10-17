//! [`PerCallTool`]: a tool rebuilt around each call's turn.
//!
//! Some tools reach the host through an object they are constructed with and
//! call without a run context: the network tools' `NetGate::disclose` and the
//! x402 payment hook live in `tinytools_std`, whose traits take no context. For
//! those, the only explicit way to give the object the turn's chat routing is to
//! build it per call. This wrapper holds a prototype that answers every static
//! `Tool` question (name, schema, permission, ...), and builds a fresh tool from
//! the call's [`ApprovalChatContext`] for each execution.
//!
//! Construction of a network tool is a handful of field copies, so building one
//! per call costs nothing next to the request it makes.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinytools::{
    PermissionLevel, Tool, ToolCallOptions, ToolCategory, ToolExposure, ToolInjectedArgument,
    ToolPolicy, ToolResult, ToolRunContext, ToolScope, ToolSpec, ToolTimeout,
};

use crate::agent::tinyagents::host::run_context::{approval_for_tool, dispatch_ctx_for_tool};
use crate::core::runtime::DispatchCtx;
use crate::security::approval::ApprovalChatContext;

/// Builds the tool one call runs, from that call's chat routing and the
/// dispatch context of the run it executes in.
type Build<T> = Arc<dyn Fn(Option<&ApprovalChatContext>, &DispatchCtx) -> T + Send + Sync>;

/// A tool whose executing instance is built for each call.
pub struct PerCallTool<T: Tool> {
    prototype: T,
    build: Build<T>,
}

impl<T: Tool> PerCallTool<T> {
    /// `build` makes the tool for one call; the prototype is built with no chat
    /// and an unbound context and answers every question that does not execute.
    pub fn new(
        build: impl Fn(Option<&ApprovalChatContext>, &DispatchCtx) -> T + Send + Sync + 'static,
    ) -> Self {
        let prototype = build(None, &DispatchCtx::unbound());
        Self {
            prototype,
            build: Arc::new(build),
        }
    }
}

impl<T: Tool> std::fmt::Debug for PerCallTool<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("PerCallTool")
            .field(&self.prototype.name())
            .finish()
    }
}

#[async_trait]
impl<T: Tool> Tool for PerCallTool<T> {
    fn name(&self) -> &str {
        self.prototype.name()
    }

    fn description(&self) -> &str {
        self.prototype.description()
    }

    fn parameters_schema(&self) -> Value {
        self.prototype.parameters_schema()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        (self.build)(None, &dispatch_ctx_for_tool(None))
            .execute(args)
            .await
    }

    async fn prepare_authorization(&self, args: &Value) -> anyhow::Result<Option<Value>> {
        self.prototype.prepare_authorization(args).await
    }

    async fn execute_with_options(
        &self,
        args: Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        (self.build)(None, &dispatch_ctx_for_tool(None))
            .execute_with_options(args, options)
            .await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let approval = approval_for_tool(context);
        (self.build)(approval.chat.as_ref(), &dispatch_ctx_for_tool(context))
            .execute_with_context(args, options, context)
            .await
    }

    fn policy(&self) -> ToolPolicy {
        self.prototype.policy()
    }

    fn injected_arguments(&self) -> Vec<ToolInjectedArgument> {
        self.prototype.injected_arguments()
    }

    fn supports_markdown(&self) -> bool {
        self.prototype.supports_markdown()
    }

    fn permission_level(&self) -> PermissionLevel {
        self.prototype.permission_level()
    }

    fn permission_level_with_args(&self, args: &Value) -> PermissionLevel {
        self.prototype.permission_level_with_args(args)
    }

    fn scope(&self) -> ToolScope {
        self.prototype.scope()
    }

    fn category(&self) -> ToolCategory {
        self.prototype.category()
    }

    fn exposure(&self) -> ToolExposure {
        self.prototype.exposure()
    }

    fn family(&self) -> Option<&str> {
        self.prototype.family()
    }

    fn tags(&self) -> Vec<String> {
        self.prototype.tags()
    }

    fn indirect_target(&self, args: &Value) -> Option<tinytools::rules::IndirectCall> {
        self.prototype.indirect_target(args)
    }

    async fn indirect_target_async(
        &self,
        args: &Value,
    ) -> anyhow::Result<Option<tinytools::rules::IndirectCall>> {
        self.prototype.indirect_target_async(args).await
    }

    fn is_concurrency_safe(&self, args: &Value) -> bool {
        self.prototype.is_concurrency_safe(args)
    }

    fn external_effect(&self) -> bool {
        self.prototype.external_effect()
    }

    fn external_effect_with_args(&self, args: &Value) -> bool {
        self.prototype.external_effect_with_args(args)
    }

    fn max_result_size_chars(&self) -> Option<usize> {
        self.prototype.max_result_size_chars()
    }

    fn timeout_policy(&self, args: &Value) -> ToolTimeout {
        self.prototype.timeout_policy(args)
    }

    fn host_extension(&self) -> Option<&(dyn Any + Send + Sync)> {
        self.prototype.host_extension()
    }

    fn host_call_extension(&self, args: &Value) -> Option<Box<dyn Any + Send + Sync>> {
        self.prototype.host_call_extension(args)
    }

    fn spec(&self) -> ToolSpec {
        self.prototype.spec()
    }

    fn display_label(&self, args: &Value) -> Option<String> {
        self.prototype.display_label(args)
    }

    fn display_detail(&self, args: &Value) -> Option<String> {
        self.prototype.display_detail(args)
    }

    fn return_direct(&self) -> bool {
        self.prototype.return_direct()
    }
}

#[cfg(test)]
#[path = "per_call_tests.rs"]
mod tests;

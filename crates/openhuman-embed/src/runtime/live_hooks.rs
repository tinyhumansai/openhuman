//! Tracked runtime hooks, composed with agent-local hooks.
use super::Runtime;
use crate::seams::{PostTurnHook, ToolHook, ToolHookContext, ToolHookDecision, TurnContext};
use std::sync::Arc;

pub(super) struct NamedPostTurnHook(pub String, pub Arc<dyn PostTurnHook>);
#[async_trait::async_trait]
impl PostTurnHook for NamedPostTurnHook {
    fn name(&self) -> &str {
        &self.0
    }
    async fn on_turn_complete(&self, context: &TurnContext) -> anyhow::Result<()> {
        self.1.on_turn_complete(context).await
    }
}
pub(super) struct NamedToolHook(pub String, pub Arc<dyn ToolHook>);
#[async_trait::async_trait]
impl ToolHook for NamedToolHook {
    fn name(&self) -> &str {
        &self.0
    }
    async fn before_tool(&self, context: &ToolHookContext) -> anyhow::Result<()> {
        self.1.before_tool(context).await
    }
    async fn after_tool(&self, context: &ToolHookContext) -> anyhow::Result<()> {
        self.1.after_tool(context).await
    }
    async fn before_tool_decision(&self, context: &ToolHookContext) -> ToolHookDecision {
        self.1.before_tool_decision(context).await
    }
    async fn before_tool_decision_in(
        &self,
        dispatch: &openhuman_core::core::runtime::DispatchCtx,
        context: &ToolHookContext,
    ) -> ToolHookDecision {
        self.1.before_tool_decision_in(dispatch, context).await
    }
    async fn after_tool_context(&self, context: &ToolHookContext) -> Option<String> {
        self.1.after_tool_context(context).await
    }
}
impl Runtime {
    /// Add, replace, or remove a runtime-wide post-turn hook by name.
    /// This also replaces hooks installed by the builder. Changes apply to
    /// subsequently assembled turns; removal restores any predecessor hook.
    pub fn post_turn_hook(&self, name: &str, hook: Option<Arc<dyn crate::seams::PostTurnHook>>) {
        let context = self.core_runtime().context().clone();
        openhuman_core::core::runtime::CoreContext::sync_scope(context.clone(), || {
            let mut seams = self
                .guard
                .seams
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(seams) = seams.as_mut() {
                seams.post_turn_hook(name, hook);
            } else if let Some(overrides) = context.host_overrides() {
                overrides.post_turn_hook(name, hook);
            }
        });
    }
    /// Add, replace, or remove a runtime-wide tool hook by name.
    /// This also replaces hooks installed by the builder. Changes apply to
    /// subsequently assembled turns; removal restores any predecessor hook.
    pub fn tool_hook(&self, name: &str, hook: Option<Arc<dyn crate::seams::ToolHook>>) {
        let context = self.core_runtime().context().clone();
        openhuman_core::core::runtime::CoreContext::sync_scope(context.clone(), || {
            let mut seams = self
                .guard
                .seams
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(seams) = seams.as_mut() {
                seams.tool_hook(name, hook);
            } else if let Some(overrides) = context.host_overrides() {
                overrides.tool_hook(name, hook);
            }
        });
    }
}

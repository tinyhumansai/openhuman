//! Inline host approval carried by the agent/turn tool-hook scope.

use crate::seams::{ToolHook, ToolHookContext, ToolHookDecision};
/// A sendable asynchronous permission decision borrowing its tool context.
pub type PermissionFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = ToolHookDecision> + Send + 'a>>;

pub(crate) struct PermissionHook<F> {
    name: String,
    callback: F,
}

impl<F> PermissionHook<F> {
    pub(crate) fn new(callback: F) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        Self {
            // Agent-local overrides replace hooks by name. Each permission
            // registration must survive, in registration order.
            name: format!(
                "embed.can_use_tool.{:020}",
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ),
            callback,
        }
    }
}

#[async_trait::async_trait]
impl<F> ToolHook for PermissionHook<F>
where
    F: for<'a> Fn(&'a ToolHookContext) -> PermissionFuture<'a> + Send + Sync,
{
    fn name(&self) -> &str {
        &self.name
    }
    async fn before_tool(&self, _: &ToolHookContext) -> anyhow::Result<()> {
        Ok(())
    }
    async fn after_tool(&self, _: &ToolHookContext) -> anyhow::Result<()> {
        Ok(())
    }
    async fn before_tool_decision(&self, context: &ToolHookContext) -> ToolHookDecision {
        (self.callback)(context).await
    }
}

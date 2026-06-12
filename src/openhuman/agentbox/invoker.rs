//! Bridges AgentBox `/run` invocations to OpenHuman's agent runtime.
//!
//! The real bridge to the agent runtime lands in Task 9; until then,
//! [`CoreAgentInvoker`] fails loudly so a wrongly deployed early build
//! cannot silently no-op.

use async_trait::async_trait;
use std::sync::Arc;

/// Bridges AgentBox `/run` invocations to OpenHuman's agent runtime.
///
/// Implementations resolve (or create) a thread, drive a single user turn
/// through the full agent runtime (skills, tools, memory), and return the
/// final assistant text + the thread id used.
#[async_trait]
pub trait AgentInvoker: Send + Sync + 'static {
    async fn invoke(
        &self,
        thread_id: Option<&str>,
        message: &str,
    ) -> Result<InvocationOutput, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationOutput {
    pub assistant_message: String,
    pub thread_id: String,
}

/// Production impl — wired to the real agent runtime in Task 9.
#[derive(Default)]
pub struct CoreAgentInvoker;

#[async_trait]
impl AgentInvoker for CoreAgentInvoker {
    async fn invoke(
        &self,
        _thread_id: Option<&str>,
        _message: &str,
    ) -> Result<InvocationOutput, String> {
        Err("agentbox: agent runtime bridge not wired (Task 9)".into())
    }
}

/// Convenience alias used by the rest of the module.
pub type SharedInvoker = Arc<dyn AgentInvoker>;

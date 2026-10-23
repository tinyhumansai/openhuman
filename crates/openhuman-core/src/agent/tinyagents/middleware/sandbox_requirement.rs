//! Fold declared sandbox requirements into the single TinySecurity tool gate.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tinyagents_harness::context::RunContext;
use tinyagents_harness::middleware::{PolicyDecision, ToolCallPolicy};
use tinyinference_llm::tool::ToolCall as TaToolCall;
use tinytools::{SandboxMode, ToolPolicy};

use crate::agent::tinyagents::host::OpenHumanRunContext;

/// The host adapter for the SDK's declarative sandbox requirement.
///
/// Actual sandbox resolution and process planning belongs to TinySecurity; this
/// adapter only prevents a tool that declares `sandbox = Required` from running
/// in a turn whose workspace does not carry that requirement. Keeping it in the
/// TinySecurity gate means every host invocation path enforces the same rule.
pub(crate) struct SandboxRequirementPolicy {
    policies: HashMap<String, ToolPolicy>,
}

impl SandboxRequirementPolicy {
    pub(crate) fn from_tool_sets(tool_sets: &[Arc<Vec<Box<dyn tinytools::Tool>>>]) -> Self {
        let policies = tool_sets
            .iter()
            .flat_map(|set| set.iter())
            .map(|tool| (tool.name().to_string(), tool.policy()))
            .collect();
        Self { policies }
    }

    #[cfg(test)]
    pub(crate) fn new(policies: HashMap<String, ToolPolicy>) -> Self {
        Self { policies }
    }
}

#[async_trait]
impl ToolCallPolicy<OpenHumanRunContext> for SandboxRequirementPolicy {
    fn name(&self) -> &str {
        "sandbox_requirement"
    }

    async fn check(
        &self,
        ctx: &RunContext<OpenHumanRunContext>,
        call: &TaToolCall,
    ) -> PolicyDecision {
        let Some(policy) = self.policies.get(&call.name) else {
            return PolicyDecision::Allow;
        };
        if policy.runtime.sandbox != SandboxMode::Required {
            return PolicyDecision::Allow;
        }
        let sandboxed = ctx
            .workspace
            .as_ref()
            .is_some_and(|workspace| workspace.sandbox == SandboxMode::Required);
        if sandboxed {
            PolicyDecision::Allow
        } else {
            PolicyDecision::deny(format!(
                "tool `{}` requires a sandbox but the run has none",
                call.name
            ))
        }
    }
}

#[cfg(test)]
#[path = "sandbox_requirement_tests.rs"]
mod tests;

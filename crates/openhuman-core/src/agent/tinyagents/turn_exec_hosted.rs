//! Process-shared hosted harness used only for root agent invocations.

use async_trait::async_trait;
use std::sync::LazyLock;
use tinyagents_harness::host::{ContextComposer, TurnContextRequest};
use tinyagents_harness::runtime::AgentHarness;

use crate::agent::tinyagents::host::OpenHumanRunContext;

static ROOT_HOSTED_HARNESS: LazyLock<AgentHarness<(), OpenHumanRunContext>> =
    LazyLock::new(AgentHarness::new);

pub(super) fn root_hosted_harness() -> &'static AgentHarness<(), OpenHumanRunContext> {
    &ROOT_HOSTED_HARNESS
}

/// Keeps the session's already-built system/context ladder authoritative when
/// crossing the hosted invocation boundary.
pub(super) struct PrecomposedRootContext;

#[async_trait]
impl ContextComposer for PrecomposedRootContext {
    async fn compose_system_prompt(
        &self,
        _: &TurnContextRequest,
    ) -> tinyagents_harness::Result<String> {
        Ok(String::new())
    }

    async fn preamble(
        &self,
        _: &TurnContextRequest,
    ) -> tinyagents_harness::Result<Vec<tinyinference_llm::message::Message>> {
        Ok(Vec::new())
    }
}

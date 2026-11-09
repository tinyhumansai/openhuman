//! Shared, stateless harness used for durable hosted root turns.

use std::sync::LazyLock;
use tinyagents_harness::runtime::AgentHarness;

use crate::agent::tinyagents::host::OpenHumanRunContext;

/// The durable root entry point carries no product authority. Models, tools,
/// middleware, progress, and host capabilities are attached per invocation,
/// preventing concurrent sessions from replacing one another's host state.
static ROOT_HOSTED_HARNESS: LazyLock<AgentHarness<(), OpenHumanRunContext>> =
    LazyLock::new(AgentHarness::new);

pub(super) fn root_hosted_harness() -> &'static AgentHarness<(), OpenHumanRunContext> {
    &ROOT_HOSTED_HARNESS
}

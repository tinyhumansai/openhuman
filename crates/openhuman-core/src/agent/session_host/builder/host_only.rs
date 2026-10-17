//! Host-only sessions: the host's belt is the whole belt.
//!
//! [`HostTurnTools`](super::HostTurnTools) adds a host's tools *beside* the
//! config-derived registry; a withheld config tool stays registered. That is
//! the right default for an embedder extending an agent, and the wrong one for
//! a host that hands an agent read-only tools over untrusted input (a PR-review
//! bot reading a diff) and must be certain the model can never act.
//!
//! [`OpenHumanSessionHost::from_config_host_only`] builds that session. Three
//! independent layers, so no single mistake re-opens the surface:
//!
//! 1. **Registry.** No config-derived tool is constructed and no delegation
//!    tool is synthesised; the belt is the host's tools and nothing else.
//! 2. **Policy.** [`HostOnlyToolPolicy`] denies every call whose name is not
//!    one of the host's, before the host's own gate is consulted, so even a
//!    model that names a built-in tool is refused.
//! 3. **Definition.** The session runs a narrowed copy of the definition
//!    ([`host_only_definition`]): no sub-agents (so the per-turn delegation
//!    refresh has nothing to add), no deferred tools, read-only sandbox, and
//!    no memory context.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;

use crate::agent::harness::host_definition::AgentDefinition;
use tinyagents_harness::context::RunContext;
use tinyagents_harness::middleware::{PolicyDecision, ToolCallPolicy};
use tinyinference_llm::tool::ToolCall;

use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::agent::OpenHumanSessionHost;
use crate::config::Config;
use anyhow::Result;
use tinyagents_definition::{SandboxMode, ToolScope};

/// Deny-by-default gate for a host-only session.
///
/// A name outside `allowed` is refused outright. A name inside it is then put
/// to the host's own gate, when the host supplied one, so host-only narrows a
/// host's policy and never widens it.
pub struct HostOnlyToolPolicy {
    allowed: HashSet<String>,
    host: Option<
        Arc<
            dyn tinyagents_harness::middleware::ToolCallPolicy<
                crate::agent::tinyagents::host::OpenHumanRunContext,
            >,
        >,
    >,
}

impl HostOnlyToolPolicy {
    /// Admit exactly `allowed`, subject to `host`'s own decision.
    #[must_use]
    pub fn new(
        allowed: HashSet<String>,
        host: Option<
            Arc<
                dyn tinyagents_harness::middleware::ToolCallPolicy<
                    crate::agent::tinyagents::host::OpenHumanRunContext,
                >,
            >,
        >,
    ) -> Self {
        Self { allowed, host }
    }
}

#[async_trait]
impl ToolCallPolicy<OpenHumanRunContext> for HostOnlyToolPolicy {
    fn name(&self) -> &str {
        "host_only"
    }

    async fn check(
        &self,
        ctx: &RunContext<OpenHumanRunContext>,
        call: &ToolCall,
    ) -> PolicyDecision {
        if !self.allowed.contains(&call.name) {
            log::debug!(
                "[agent::host_only] denied tool outside the host belt: {}",
                call.name
            );
            return PolicyDecision::deny(format!(
                "tool `{}` is not available to this agent",
                call.name
            ));
        }
        match &self.host {
            Some(host) => host.check(ctx, call).await,
            None => PolicyDecision::Allow,
        }
    }
}

/// The visible set of a host-only belt before the host's names are merged in:
/// explicitly zero tools (an empty set would mean "no filter").
pub(super) fn empty_belt() -> HashSet<String> {
    HashSet::from([crate::agent::harness::host_definition::NO_TOOLS_SENTINEL.to_string()])
}

/// The session gate: on a host-only belt, [`HostOnlyToolPolicy`] over the
/// belt's names (`tools` holds only the host's by then) wrapping the host's
/// own gate; otherwise the host's gate unchanged. The recovery tool and every
/// other config-side addition are kept off the belt by the caller.
pub(super) fn session_policy(
    host_only: bool,
    tools: &[Box<dyn tinytools::Tool>],
    host: Option<
        Arc<
            dyn tinyagents_harness::middleware::ToolCallPolicy<
                crate::agent::tinyagents::host::OpenHumanRunContext,
            >,
        >,
    >,
) -> Option<
    Arc<
        dyn tinyagents_harness::middleware::ToolCallPolicy<
            crate::agent::tinyagents::host::OpenHumanRunContext,
        >,
    >,
> {
    if !host_only {
        return host;
    }
    let allowed = tools.iter().map(|tool| tool.name().to_string()).collect();
    Some(Arc::new(HostOnlyToolPolicy::new(allowed, host)))
}

/// The definition a host-only session runs under: `definition` with every
/// route to a tool the host did not supply removed.
pub(super) fn host_only_definition(definition: &AgentDefinition) -> AgentDefinition {
    let mut def = definition.clone();
    // An empty named belt is zero tools; the host's names join it as scope
    // additions when the belt is merged.
    def.tools = ToolScope::Named(Vec::new());
    def.extra_tools.clear();
    def.deferred_tools.clear();
    def.skill_filter = None;
    // Without sub-agents nothing is synthesised at build time, and
    // `refresh_delegation_tools` returns before adding any later.
    def.subagents.clear();
    def.sandbox_mode = SandboxMode::ReadOnly;
    def.extensions.omit_memory_context = true;
    def
}

impl OpenHumanSessionHost {
    /// A session whose whole tool belt is what `host` supplies.
    ///
    /// See the module docs for the three layers this applies. `host` may be
    /// `None`, which is a session with no tools at all. The `definition`'s
    /// tool scope, sub-agents and sandbox are ignored: host-only overrides
    /// them.
    ///
    /// # Errors
    ///
    /// As [`OpenHumanSessionHost::from_config_with_definition`].
    pub fn from_config_host_only(
        ctx: &crate::core::runtime::DispatchCtx,
        config: &Config,
        definition: &AgentDefinition,
        host: Option<&super::HostTools>,
        session_id: Option<&str>,
    ) -> Result<Self> {
        let definition = host_only_definition(definition);
        let mut agent = OpenHumanSessionHost::build_session_agent_inner(
            ctx,
            config,
            &definition.id,
            Some(&definition),
            true,
            host,
            session_id,
        )?;
        agent.host_only = true;
        Ok(agent)
    }

    /// Treat the messages this session runs as untrusted data: skip the
    /// prompt-injection guard in [`Self::run_single`].
    ///
    /// A reviewer has to read a PR diff that says "ignore previous
    /// instructions"; the guard would refuse it. Only a host-only session
    /// may, because it has nothing it could be talked into doing.
    ///
    /// # Errors
    ///
    /// When `untrusted` is set on a session not built host-only.
    pub fn set_untrusted_input(&mut self, untrusted: bool) -> Result<()> {
        anyhow::ensure!(
            !untrusted || self.host_only,
            "untrusted input is only allowed on a host-only session"
        );
        self.untrusted_input = untrusted;
        Ok(())
    }
}

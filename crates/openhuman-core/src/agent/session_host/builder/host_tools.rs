//! Host-supplied tools a session is built with, beside the ones config names.
//!
//! A session host built through [`OpenHumanSessionHost::builder`] takes a tool
//! belt directly — an embedder hands it `Box<dyn Tool>` objects and they are
//! the belt. A session built *from config*
//! ([`from_config_with_definition`](super::super::OpenHumanSessionHost::from_config_with_definition))
//! cannot: it is reconstructed on every turn from `Config` and an
//! `AgentDefinition`, both of which are data, so nothing carrying a `dyn Tool`
//! survives between turns. That is why an `openhuman_embed::Agent` has only
//! ever reached a host's own tools over MCP.
//!
//! [`HostTools`] is the seam that closes it, and it is a **factory rather than
//! a belt** for exactly the reason above: `Box<dyn Tool>` is not `Clone` and
//! `Agent` is, so a stored belt could not survive the per-turn rebuild. The
//! closure is invoked once per turn, which also means the belt it returns may
//! differ from turn to turn — a host whose tools are bound to something
//! shorter-lived than the agent (one episode, one room, one assignment) can
//! express that here instead of registering a second agent for it.
//!
//! The prompt's tool catalogue is rendered from the same belt in the same
//! build, so a changing belt and its description stay consistent **on a turn
//! that composes its prompt**. A resumed session reuses its persisted system
//! messages, so a belt that moves under one is described by the prompt it had
//! when the thread opened; a host that varies its belt should run such turns
//! on a session of their own. Permanent attachments are the exception: their
//! catalogue occupies a managed system section refreshed independently of
//! the frozen host prompt.

use std::collections::HashSet;
use std::sync::Arc;

use tinytools::Tool;

use crate::agent::OpenHumanSessionHost;
use crate::config::Config;
use anyhow::Result;

/// One turn's worth of host-supplied belt.
///
/// `visible` adds names to a named session allow-list. A wildcard session
/// retains its native registry and applies each tool's exposure metadata;
/// `permanent` explicitly forces selected host tools into the prompt and schema.
/// `policy`, when set, becomes the whole session's gate; see
/// [`with_policy`](Self::with_policy).
#[derive(Default)]
pub struct HostTurnTools {
    /// The tools themselves, placed **ahead of** the config-derived belt so a
    /// host tool wins a collision on its name.
    pub tools: Vec<Box<dyn Tool>>,
    /// Tool names permanently advertised directly and rendered in a managed prompt section.
    pub permanent: HashSet<String>,
    /// Names to add to a named provider-visible allow-list.
    /// Wildcard sessions already expand the registry subject to tool exposure.
    pub visible: HashSet<String>,
    /// Names to remove from the provider-visible allow-list for this turn.
    ///
    /// This is applied after the config-derived scope and `visible` names are
    /// combined, so a host can hide a tool that the agent normally receives
    /// without changing the agent's scope for other turns. Withheld tools
    /// remain in the registry; hosts that require a call boundary must also
    /// enforce one through the session's tool policy.
    pub withheld: HashSet<String>,
    /// The session's admission gate, if the host sets one.
    pub policy: Option<
        Arc<
            dyn tinyagents_harness::middleware::ToolCallPolicy<
                crate::agent::tinyagents::host::OpenHumanRunContext,
            >,
        >,
    >,
}

impl HostTurnTools {
    /// A belt with every tool advertised, which is the common case.
    #[must_use]
    pub fn advertised(tools: Vec<Box<dyn Tool>>) -> Self {
        let visible = tools.iter().map(|tool| tool.name().to_string()).collect();
        Self {
            tools,
            permanent: HashSet::new(),
            visible,
            withheld: HashSet::new(),
            policy: None,
        }
    }

    /// Fold this belt into the session's, and hand back the gate it carries.
    ///
    /// # Host-first, deliberately
    ///
    /// The host's tools are spliced in **ahead of** the config-derived ones so
    /// a host tool wins a collision on its name: the host named this object
    /// specifically, and a host that cannot override a tool it collides with
    /// has no way to correct one.
    ///
    /// First and not last because TinyAgents' `dedup_visible_tool_specs` keeps the first
    /// occurrence. Appending would advertise the config-derived spec while the
    /// host believed it had replaced it -- the model told about one tool and a
    /// different one answering, which is the version of this bug that is
    /// hardest to see from outside.
    ///
    /// Returns `None` for an empty belt, which is also a host that supplied no
    /// gate: there is nothing to admit.
    ///
    /// Tool schema deduplication is owned by
    /// `tinyagents_runtime::dedup_visible_tool_specs`.
    pub(super) fn merge_into(
        self,
        agent_id: &str,
        tools: &mut Vec<Box<dyn Tool>>,
        visible: &mut HashSet<String>,
    ) -> Result<MergedHostTurnTools> {
        if self.is_empty() {
            return Ok(MergedHostTurnTools {
                policy: None,
                withheld: HashSet::new(),
                permanent: HashSet::new(),
                scope_additions: HashSet::new(),
            });
        }
        log::debug!(
            "[agent::builder] host supplied {} tool(s) for agent_id={agent_id}: {:?}",
            self.tools.len(),
            self.tools
                .iter()
                .map(|tool| tool.name())
                .collect::<Vec<_>>(),
        );
        for name in &self.permanent {
            anyhow::ensure!(
                !tools.iter().any(|tool| tool.name() == name),
                "permanent tool name collision: {name}"
            );
            anyhow::ensure!(
                self.tools.iter().filter(|tool| tool.name() == name).count() == 1,
                "permanent tool must have exactly one source: {name}"
            );
        }
        let permanent = &self.permanent;
        tools.splice(
            0..0,
            self.tools.into_iter().map(|tool| {
                if permanent.contains(tool.name()) {
                    Box::new(tinyagents_runtime::PermanentTool(tool)) as Box<dyn Tool>
                } else {
                    tool
                }
            }),
        );
        // An empty set represents the original wildcard scope until build().
        // Filling it with an attached source's names would turn it into a
        // literal allowlist and silently discard the native registry.
        if !visible.is_empty() {
            visible.extend(self.visible.iter().cloned());
        }
        let scope_additions = self.visible.difference(&self.withheld).cloned().collect();
        Ok(MergedHostTurnTools {
            policy: self.policy,
            withheld: self.withheld,
            permanent: self.permanent,
            scope_additions,
        })
    }

    /// Sets the gate for the whole session.
    ///
    /// # This replaces; it does not wrap
    ///
    /// The policy set here becomes the session's tool policy outright -- it is
    /// not consulted first and then deferred to a config-derived one, because
    /// there is no composition step to defer through.
    ///
    /// That is what the episode case wants: a gate saying *admit my belt, and
    /// ask me about everything else* is a statement about the whole session,
    /// not only about the tools the host supplied. But it means **a host that
    /// gates only its own names denies every other tool on the belt**. If the
    /// session should keep an existing policy for calls the host does not own,
    /// the host composes the two and passes the result here.
    #[must_use]
    pub fn with_policy(
        mut self,
        policy: Arc<
            dyn tinyagents_harness::middleware::ToolCallPolicy<
                crate::agent::tinyagents::host::OpenHumanRunContext,
            >,
        >,
    ) -> Self {
        self.policy = Some(policy);
        self
    }

    /// Whether this contributes nothing, so a caller can skip the union.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
            && self.visible.is_empty()
            && self.withheld.is_empty()
            && self.policy.is_none()
            && self.permanent.is_empty()
    }
}

/// The parts of a host turn belt that the session builder applies after
/// expanding the agent's static scope.
pub(super) struct MergedHostTurnTools {
    pub policy: Option<
        Arc<
            dyn tinyagents_harness::middleware::ToolCallPolicy<
                crate::agent::tinyagents::host::OpenHumanRunContext,
            >,
        >,
    >,
    pub withheld: HashSet<String>,
    pub permanent: HashSet<String>,
    pub scope_additions: HashSet<String>,
}

pub(super) fn add_permanent_tools(
    mut definition: crate::agent_registry::host_definition::AgentDefinition,
    permanent: &HashSet<String>,
) -> crate::agent_registry::host_definition::AgentDefinition {
    definition.extra_tools.extend(permanent.iter().cloned());
    definition.extra_tools.sort();
    definition.extra_tools.dedup();
    definition
}

pub(super) fn scope_def(
    definition: Option<&crate::agent_registry::host_definition::AgentDefinition>,
    host_tools: &MergedHostTurnTools,
) -> Option<crate::agent_registry::host_definition::AgentDefinition> {
    let mut definition = definition.cloned()?;
    if let tinyagents_definition::ToolScope::Named(names) = &mut definition.tools {
        names.retain(|name| !host_tools.withheld.contains(name));
        definition
            .extra_tools
            .retain(|name| !host_tools.withheld.contains(name));
        definition
            .extra_tools
            .extend(host_tools.scope_additions.iter().cloned());
        definition.extra_tools.sort();
        definition.extra_tools.dedup();
    }
    Some(definition)
}

pub(super) async fn merge_for_turn(
    dispatch: &crate::core::runtime::DispatchCtx,
    host: Option<&HostTools>,
    agent_id: &str,
    session_id: Option<&str>,
    tools: &mut Vec<Box<dyn Tool>>,
    visible: &mut HashSet<String>,
) -> Result<MergedHostTurnTools> {
    let host_tools = match host {
        Some(build) => {
            Some(build(TurnContext::new(agent_id, session_id).with_dispatch(dispatch)).await)
        }
        None => None,
    };
    match host_tools.map(|host_tools| host_tools.merge_into(agent_id, tools, visible)) {
        Some(merged) => merged,
        None => Ok(MergedHostTurnTools {
            policy: None,
            withheld: HashSet::new(),
            permanent: HashSet::new(),
            scope_additions: HashSet::new(),
        }),
    }
}

impl std::fmt::Debug for HostTurnTools {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostTurnTools")
            .field(
                "tools",
                &self
                    .tools
                    .iter()
                    .map(|tool| tool.name())
                    .collect::<Vec<_>>(),
            )
            .field("visible", &self.visible)
            .field("withheld", &self.withheld)
            .field("policy", &self.policy.is_some())
            .finish()
    }
}

/// What the turn being built is, as far as a host's tool factory needs to know.
///
/// A belt that varies has to vary on *something*. Without this the factory is
/// called with no argument and has to infer its own occasion from state it
/// closed over, which works only while the agent serves one conversation at a
/// time -- the moment it serves two, a belt bound to "the current episode" is
/// a race rather than a decision.
///
/// Non-exhaustive: this describes an occasion, and occasions gain detail.
/// Construct it with [`TurnContext::new`] and read it through the accessors so
/// a later field cannot break a host that matched on it.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct TurnContext<'a> {
    agent_id: &'a str,
    session_id: Option<&'a str>,
    dispatch: Option<&'a crate::core::runtime::DispatchCtx>,
}

impl std::fmt::Debug for TurnContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnContext")
            .field("agent_id", &self.agent_id)
            .field("session_id", &self.session_id)
            .field("dispatch", &self.dispatch.is_some())
            .finish()
    }
}

impl<'a> TurnContext<'a> {
    /// The turn's occasion: which agent, and which conversation if one was named.
    #[must_use]
    pub fn new(agent_id: &'a str, session_id: Option<&'a str>) -> Self {
        Self {
            agent_id,
            session_id,
            dispatch: None,
        }
    }

    /// The same occasion, built under `dispatch`: the context of the session
    /// being assembled, which a belt that reads the acting tenant takes it from.
    #[must_use]
    pub fn with_dispatch(mut self, dispatch: &'a crate::core::runtime::DispatchCtx) -> Self {
        self.dispatch = Some(dispatch);
        self
    }

    /// The context this turn is built under: the one named by
    /// [`with_dispatch`](Self::with_dispatch), else the calling task's own (an
    /// occasion built without one).
    #[must_use]
    pub fn dispatch_ctx(&self) -> crate::core::runtime::DispatchCtx {
        self.dispatch
            .cloned()
            .unwrap_or_else(|| crate::core::runtime::DispatchCtx::from_weak(None))
    }

    /// The agent definition this turn runs as.
    #[must_use]
    pub fn agent_id(&self) -> &'a str {
        self.agent_id
    }

    /// The conversation this turn runs in, as the caller named it.
    ///
    /// `None` when no session was named -- a one-shot turn, or a path that
    /// mints an id only after the session is built. A host keying its belt on
    /// this should decide what an unnamed turn gets rather than assume it
    /// cannot happen.
    #[must_use]
    pub fn session_id(&self) -> Option<&'a str> {
        self.session_id
    }
}

/// Builds one turn's host belt.
///
/// Invoked once per **session build**, which on the paths a host reaches --
/// `agent_chat` builds a session for every turn -- means once per turn. The
/// distinction matters for anything that composes sessions differently: the
/// guarantee is per build, not per turn, and a build that is reused serves the
/// belt it was built with.
///
/// See [`TurnContext`] for what the factory is told about the occasion.
pub type HostTools = Arc<
    dyn for<'a> Fn(TurnContext<'a>) -> futures::future::BoxFuture<'a, HostTurnTools> + Send + Sync,
>;

/// The constructor the host-tools seam exists for, kept beside the types it
/// takes rather than with the config-derived constructors it sits among.
impl OpenHumanSessionHost {
    /// [`OpenHumanSessionHost::from_config_with_definition`], plus a belt the host supplies
    /// itself.
    ///
    /// The seam an embedder needs to put its **own** `dyn Tool` on an agent it
    /// configures through data. Everything else on this path is reconstructed
    /// from `Config` and the definition on every turn, so a host that owned a
    /// tool object had nowhere to put it and reached its tools over MCP
    /// instead — paying a discovery turn, an opaque `arguments` object the
    /// provider cannot validate, and a prompt section explaining the envelope.
    ///
    /// `host` is a factory rather than a belt because this constructor runs
    /// once per turn and `Box<dyn Tool>` is not `Clone`. A host may therefore
    /// return a different belt each time; see [`HostTurnTools`] for what that
    /// does and does not keep consistent with the prompt.
    ///
    /// # Errors
    ///
    /// As [`OpenHumanSessionHost::from_config_with_definition`].
    ///
    /// `session_id` is the conversation this turn runs in, reaching the
    /// factory as [`TurnContext::session_id`]; `None` when none is named yet.
    pub async fn from_config_with_host_tools(
        ctx: &crate::core::runtime::DispatchCtx,
        config: &Config,
        definition: &crate::agent_registry::host_definition::AgentDefinition,
        host: &super::HostTools,
        session_id: Option<&str>,
    ) -> Result<Self> {
        OpenHumanSessionHost::build_session_agent_inner(
            ctx,
            config,
            &definition.id,
            Some(definition),
            false,
            Some(host),
            session_id,
        )
        .await
    }

    /// Build a session agent from a definition the caller already holds,
    /// rather than an id resolved through `AgentDefinitionRegistry::current()`
    /// or `config.agent_registry`.
    ///
    /// The definition is authoritative for this agent's prompt, tool scope,
    /// sandbox mode and delegation allowlist; it is stamped on the session so
    /// in-turn readers ([`OpenHumanSessionHost::resolved_definition`]) never fall back to a
    /// registry entry that happens to share its id. This is the constructor a
    /// library host uses to run many independently specified agents on one
    /// booted core.
    pub async fn from_config_with_definition(
        ctx: &crate::core::runtime::DispatchCtx,
        config: &Config,
        definition: &crate::agent_registry::host_definition::AgentDefinition,
    ) -> Result<Self> {
        log::debug!(
            "[agent] from_config_with_definition id={} sandbox={:?}",
            definition.id,
            definition.sandbox_mode,
        );
        OpenHumanSessionHost::build_session_agent_inner(
            ctx,
            config,
            &definition.id,
            Some(definition),
            false,
            None,
            None,
        )
        .await
    }

    pub(crate) async fn from_config_with_definition_and_turn_model_source(
        ctx: &crate::core::runtime::DispatchCtx,
        config: &Config,
        definition: &crate::agent_registry::host_definition::AgentDefinition,
        source: crate::agent::tinyagents::TurnModelSource,
        model: String,
    ) -> Result<Self> {
        OpenHumanSessionHost::build_session_agent_inner_with_turn_model_source(
            ctx,
            config,
            &definition.id,
            Some(definition),
            false,
            None,
            None,
            Some((source, model)),
        )
        .await
    }
}

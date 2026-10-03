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

use crate::agent::tool_policy::ToolPolicy;
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
    pub policy: Option<Arc<dyn ToolPolicy>>,
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
    /// First and not last because [`dedup_visible_tool_specs`] keeps the first
    /// occurrence. Appending would advertise the config-derived spec while the
    /// host believed it had replaced it -- the model told about one tool and a
    /// different one answering, which is the version of this bug that is
    /// hardest to see from outside.
    ///
    /// Returns `None` for an empty belt, which is also a host that supplied no
    /// gate: there is nothing to admit.
    ///
    /// [`dedup_visible_tool_specs`]: super::dedup_visible_tool_specs
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
                    Box::new(super::permanent_tool::PermanentTool(tool)) as Box<dyn Tool>
                } else {
                    tool
                }
            }),
        );
        // An empty set represents the original wildcard scope until build().
        // Filling it with an attached source's names would turn it into a
        // literal allowlist and silently discard the native registry.
        if !visible.is_empty() {
            visible.extend(self.visible);
        }
        Ok(MergedHostTurnTools {
            policy: self.policy,
            withheld: self.withheld,
            permanent: self.permanent,
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
    pub fn with_policy(mut self, policy: Arc<dyn ToolPolicy>) -> Self {
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
    pub policy: Option<Arc<dyn ToolPolicy>>,
    pub withheld: HashSet<String>,
    pub permanent: HashSet<String>,
}

pub(super) fn merge_for_turn(
    host: Option<&HostTools>,
    agent_id: &str,
    session_id: Option<&str>,
    tools: &mut Vec<Box<dyn Tool>>,
    visible: &mut HashSet<String>,
) -> Result<MergedHostTurnTools> {
    match host
        .map(|build| build(TurnContext::new(agent_id, session_id)))
        .map(|host_tools| host_tools.merge_into(agent_id, tools, visible))
    {
        Some(merged) => merged,
        None => Ok(MergedHostTurnTools {
            policy: None,
            withheld: HashSet::new(),
            permanent: HashSet::new(),
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
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct TurnContext<'a> {
    agent_id: &'a str,
    session_id: Option<&'a str>,
}

impl<'a> TurnContext<'a> {
    /// The turn's occasion: which agent, and which conversation if one was named.
    #[must_use]
    pub fn new(agent_id: &'a str, session_id: Option<&'a str>) -> Self {
        Self {
            agent_id,
            session_id,
        }
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
pub type HostTools = Arc<dyn for<'a> Fn(TurnContext<'a>) -> HostTurnTools + Send + Sync>;

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
    pub fn from_config_with_host_tools(
        config: &Config,
        definition: &crate::agent::harness::definition::AgentDefinition,
        host: &super::HostTools,
        session_id: Option<&str>,
    ) -> Result<Self> {
        OpenHumanSessionHost::build_session_agent_inner(
            config,
            &definition.id,
            Some(definition),
            false,
            Some(host),
            session_id,
        )
    }
}

/// Compose the host belt before adding the remaining product configuration.
pub(super) fn tool_builder(
    host: Option<&HostTools>,
    agent_id: &str,
    session_id: Option<&str>,
    mut tools: Vec<Box<dyn Tool>>,
    mut visible: HashSet<String>,
) -> Result<super::super::SessionHostBuilder> {
    let merged = merge_for_turn(host, agent_id, session_id, &mut tools, &mut visible)?;
    let mut builder = OpenHumanSessionHost::builder()
        .tools(tools)
        .visible_tool_names(visible)
        .withheld_tool_names(merged.withheld)
        .permanent_tool_names(merged.permanent);
    // A supplied gate retains the existing replacement semantics.
    if let Some(policy) = merged.policy {
        builder = builder.tool_policy(policy);
    }
    Ok(builder)
}

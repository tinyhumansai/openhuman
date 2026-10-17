//! What an agent *is*: prompt, tool scope, sandbox mode, iteration cap.
//!
//! A thin re-spelling of the core's
//! [`AgentDefinition`](openhuman_core::agent::harness::host_definition::AgentDefinition)
//! so the library surface does not track that thirty-field struct
//! field-by-field. The default is the built-in orchestrator's definition
//! under the agent's own id — the same dynamic prompt and delegation
//! surface the desktop's main agent runs with — except that **every
//! registered tool is visible** ([`ToolScopeSpec::Wildcard`]). The desktop
//! orchestrator names its direct tools and delegates the rest (MCP bridge,
//! integrations) to specialists; a library agent that declared an MCP
//! server expects to call it, so the host narrows with
//! [`AgentDefinitionSpec::tools`] rather than widening.

use openhuman_core::agent::harness::host_definition::{AgentDefinition, AgentDefinitionRegistry};
use tinyagents_definition::{PromptSource, SandboxMode, ToolScope};

use super::AgentError;

/// Which tools an agent may call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolScopeSpec {
    /// Every tool the runtime registered (subject to `disallow_tools`).
    Wildcard,
    /// Exactly these tool names; unknown names are dropped at build time.
    Named(Vec<String>),
    /// The host's tools and nothing else.
    ///
    /// For an agent that must never act — an analyst reading an untrusted
    /// document. The registry the model sees is built from the tools the host
    /// supplies ([`AgentSpec::tools`](super::AgentSpec::tools),
    /// [`Agent::attach_tools`](super::Agent::attach_tools)) alone: no
    /// config-derived, delegation, memory, skill or MCP tool, and a
    /// deny-by-default gate refuses any other name the model calls. The agent
    /// also runs read-only (access tier and sandbox) whatever
    /// [`Access`](crate::Access) it was given. Host tools should themselves be
    /// read-only: host-only bounds *which* tools exist, not what they do.
    HostOnly,
}

/// How an agent's shell and file tools are confined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SandboxModeSpec {
    /// Only the access tier and path policy apply.
    #[default]
    None,
    /// Write and execute tools are filtered out.
    ReadOnly,
    /// Commands run under the platform jail or Docker backend.
    Sandboxed,
}

/// Starting definition: a named built-in or a complete host-supplied definition.
#[derive(Debug, Clone)]
pub enum DefinitionBase {
    /// Clone the named built-in definition.
    Builtin(String),
    /// Clone a complete host-supplied definition.
    Custom(Box<AgentDefinition>),
}
impl From<&str> for DefinitionBase {
    fn from(value: &str) -> Self {
        Self::Builtin(value.into())
    }
}
impl From<String> for DefinitionBase {
    fn from(value: String) -> Self {
        Self::Builtin(value)
    }
}
impl From<AgentDefinition> for DefinitionBase {
    fn from(value: AgentDefinition) -> Self {
        Self::Custom(Box::new(value))
    }
}

/// Builder for an agent's definition. See the module docs for the default.
#[derive(Debug, Clone, Default)]
pub struct AgentDefinitionSpec {
    base: Option<DefinitionBase>,
    system_prompt: Option<String>,
    /// The prompt is the whole system prompt, with nothing composed around it.
    bare_prompt: bool,
    tools: Option<ToolScopeSpec>,
    disallowed_tools: Vec<String>,
    tool_rules: Option<tinytools::ToolRules>,
    sandbox: Option<SandboxModeSpec>,
    max_iterations: Option<usize>,
    temperature: Option<f64>,
    display_name: Option<String>,
    when_to_use: Option<String>,
}

impl AgentDefinitionSpec {
    /// The orchestrator's definition, to be narrowed.
    pub fn new() -> Self {
        Self::default()
    }

    /// Start from a named built-in or a complete custom core definition.
    pub fn from_base(base: impl Into<DefinitionBase>) -> Self {
        Self {
            base: Some(base.into()),
            ..Self::default()
        }
    }

    pub(crate) fn explicit_model_defaults(&self) -> crate::ModelDefaults {
        let base = match &self.base {
            Some(DefinitionBase::Custom(definition)) => Some((**definition).clone()),
            Some(DefinitionBase::Builtin(name)) => {
                AgentDefinitionRegistry::builtins_only().get(name).cloned()
            }
            None => None,
        };
        crate::ModelDefaults {
            temperature: self
                .temperature
                .or_else(|| base.as_ref().map(|d| d.temperature)),
            max_iterations: self
                .max_iterations
                .or_else(|| base.as_ref().map(|d| d.max_iterations)),
            ..Default::default()
        }
    }

    /// Resolve unspecified fields from the runtime or a template. Tools only narrow.
    pub(crate) fn inherit(mut self, lower: Self) -> Result<Self, AgentError> {
        if let (Some(requested), Some(base)) = (&self.tools, &lower.tools) {
            let allowed = match (requested, base) {
                (_, ToolScopeSpec::Wildcard) | (ToolScopeSpec::HostOnly, _) => true,
                (ToolScopeSpec::Named(a), ToolScopeSpec::Named(b)) => {
                    a.iter().all(|n| b.contains(n))
                }
                _ => false,
            };
            if !allowed {
                return Err(AgentError::WidensRuntime(
                    "definition tool scope widens its base template".into(),
                ));
            }
        }
        self.base = self.base.or(lower.base);
        if self.system_prompt.is_none() {
            self.system_prompt = lower.system_prompt;
            self.bare_prompt = lower.bare_prompt;
        }
        self.tools = self.tools.or(lower.tools);
        self.disallowed_tools.extend(lower.disallowed_tools);
        // Tool rules must retain both layers' restrictions; core also enforces config rules.
        self.tool_rules = match (self.tool_rules, lower.tool_rules) {
            (Some(a), Some(b)) => {
                if a != b {
                    return Err(AgentError::WidensRuntime(
                        "cannot replace inherited template tool rules; narrow tool scope instead"
                            .into(),
                    ));
                }
                Some(a)
            }
            (a, b) => a.or(b),
        };
        self.sandbox = self.sandbox.or(lower.sandbox);
        self.max_iterations = self.max_iterations.or(lower.max_iterations);
        self.temperature = self.temperature.or(lower.temperature);
        self.display_name = self.display_name.or(lower.display_name);
        self.when_to_use = self.when_to_use.or(lower.when_to_use);
        Ok(self)
    }

    /// Replace the dynamic system prompt with this fixed text.
    ///
    /// The identity, memory and safety sections the orchestrator prompt
    /// carries are kept around it; only the body changes.
    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self.bare_prompt = false;
        self
    }

    /// Make `prompt` the whole system prompt, verbatim.
    ///
    /// Unlike [`system_prompt`](Self::system_prompt), no orchestrator
    /// identity, safety, tools, workspace or memory section is composed
    /// around it, and no memory context is injected into a new session. The
    /// host owns every byte the model is told. Tool schemas still travel in
    /// the request's `tools` field.
    pub fn bare_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self.bare_prompt = true;
        self
    }

    /// Whether this is a [`ToolScopeSpec::HostOnly`] definition.
    pub(crate) fn is_host_only(&self) -> bool {
        matches!(self.tools, Some(ToolScopeSpec::HostOnly))
    }

    /// Restrict which tools the agent may call.
    pub fn tools(mut self, scope: ToolScopeSpec) -> Self {
        self.tools = Some(scope);
        self
    }

    /// Hide these tools even when the scope would include them.
    pub fn disallow_tools<I, S>(mut self, tools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.disallowed_tools
            .extend(tools.into_iter().map(Into::into));
        self
    }

    /// Pattern rules narrowing which tools the agent may see and call — on
    /// the catalogue, `tool_search` and every call. Stacks with
    /// [`Self::tools`], [`Self::disallow_tools`] and the runtime config's
    /// `[tool_rules]`; it can only narrow. See [`tinytools::ToolRules`].
    pub fn tool_rules(mut self, rules: tinytools::ToolRules) -> Self {
        self.tool_rules = Some(rules);
        self
    }

    /// Confine the agent's shell and file tools.
    pub fn sandbox(mut self, mode: SandboxModeSpec) -> Self {
        self.sandbox = Some(mode);
        self
    }

    /// Cap the tool-call iterations per turn.
    pub fn max_iterations(mut self, n: usize) -> Self {
        self.max_iterations = Some(n);
        self
    }

    /// Default sampling temperature.
    pub fn temperature(mut self, t: f64) -> Self {
        self.temperature = Some(t);
        self
    }

    /// Human-readable name shown in transcripts and delegation catalogs.
    pub fn display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }

    /// One-line description of when this agent should be used.
    pub fn when_to_use(mut self, text: impl Into<String>) -> Self {
        self.when_to_use = Some(text.into());
        self
    }

    /// Materialize the core definition for agent `id`.
    pub(crate) fn into_core(self, id: &str) -> Result<AgentDefinition, AgentError> {
        let host_only = self.is_host_only();
        let registry = AgentDefinitionRegistry::builtins_only();
        let custom_base = self.base.is_some();
        let mut def = match self.base {
            Some(DefinitionBase::Custom(def)) => *def,
            Some(DefinitionBase::Builtin(name)) => registry
                .get(&name)
                .cloned()
                .ok_or(AgentError::UnknownDefinition(name))?,
            None => registry.get(ORCHESTRATOR_ID).cloned().ok_or_else(|| {
                AgentError::Invalid("built-in orchestrator definition is missing".into())
            })?,
        };
        def.id = id.to_string();
        def.display_name = Some(self.display_name.unwrap_or_else(|| id.to_string()));
        if let Some(text) = self.when_to_use {
            def.when_to_use = text;
        }
        match self.system_prompt {
            Some(prompt) if self.bare_prompt => {
                def.system_prompt = PromptSource::Verbatim(prompt);
                def.extensions.omit_identity = true;
                def.extensions.omit_safety_preamble = true;
                def.extensions.omit_memory_context = true;
            }
            Some(prompt) => def.system_prompt = PromptSource::Inline(prompt),
            // The orchestrator's dynamic prompt describes a delegation and
            // memory surface a host-only agent does not have.
            None if host_only => {
                return Err(AgentError::Invalid(
                    "a HostOnly agent needs its own prompt: set bare_prompt or system_prompt"
                        .to_string(),
                ));
            }
            None => {}
        }
        if custom_base {
            if let Some(requested) = &self.tools {
                let narrows = match (requested, &def.tools) {
                    (_, ToolScope::Wildcard) | (ToolScopeSpec::HostOnly, _) => true,
                    (ToolScopeSpec::Named(a), ToolScope::Named(b)) => {
                        a.iter().all(|name| b.contains(name))
                    }
                    _ => false,
                };
                if !narrows {
                    return Err(AgentError::WidensRuntime(
                        "tool scope widens the chosen base definition".into(),
                    ));
                }
            }
        }
        def.tools = match self.tools.unwrap_or_else(|| {
            if custom_base {
                match &def.tools {
                    ToolScope::Wildcard => ToolScopeSpec::Wildcard,
                    ToolScope::Named(names) => ToolScopeSpec::Named(names.clone()),
                }
            } else {
                ToolScopeSpec::Wildcard
            }
        }) {
            ToolScopeSpec::Wildcard => ToolScope::Wildcard,
            ToolScopeSpec::Named(names) => ToolScope::Named(names),
            // Zero tools; the host's names join at session build.
            ToolScopeSpec::HostOnly => ToolScope::Named(Vec::new()),
        };
        def.disallowed_tools.extend(self.disallowed_tools);
        if let Some(rules) = self.tool_rules {
            def.tool_rules = Some(rules);
        }
        def.sandbox_mode = match self.sandbox.unwrap_or({
            if custom_base {
                match def.sandbox_mode {
                    SandboxMode::None => SandboxModeSpec::None,
                    SandboxMode::ReadOnly => SandboxModeSpec::ReadOnly,
                    SandboxMode::Sandboxed => SandboxModeSpec::Sandboxed,
                }
            } else {
                SandboxModeSpec::None
            }
        }) {
            _ if host_only => SandboxMode::ReadOnly,
            SandboxModeSpec::None => SandboxMode::None,
            SandboxModeSpec::ReadOnly => SandboxMode::ReadOnly,
            SandboxModeSpec::Sandboxed => SandboxMode::Sandboxed,
        };
        if host_only {
            def.subagents.clear();
            def.extra_tools.clear();
            def.deferred_tools.clear();
        }
        if let Some(n) = self.max_iterations {
            def.max_iterations = n;
        }
        if let Some(t) = self.temperature {
            def.temperature = t;
        }
        log::debug!(
            "[embed][agent] definition id={id} prompt={} tools={:?} sandbox={:?} max_iterations={}",
            match &def.system_prompt {
                PromptSource::Inline(_) => "inline",
                PromptSource::File { .. } => "file",
                PromptSource::Dynamic(_) => "dynamic",
                PromptSource::Verbatim(_) => "verbatim",
            },
            def.tools,
            def.sandbox_mode,
            def.max_iterations
        );
        Ok(def)
    }
}

impl AgentDefinitionSpec {
    /// Materialize this spec as a worker sub-agent named `id`: it executes
    /// and delegates nothing further.
    pub(crate) fn into_subagent_core(self, id: &str) -> Result<AgentDefinition, AgentError> {
        let mut def = self.into_core(id)?;
        def.agent_tier = tinyagents_definition::AgentTier::Worker;
        def.subagents.clear();
        def.delegate_name = None;
        def.extensions.searches_connected_mcp = false;
        Ok(def)
    }
}

/// The built-in definition every embedded agent starts from.
const ORCHESTRATOR_ID: &str = "orchestrator";

#[cfg(test)]
#[path = "definition_tests.rs"]
mod tests;

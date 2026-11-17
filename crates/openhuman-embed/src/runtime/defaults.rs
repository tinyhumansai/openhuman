//! Agent defaults resolve as Config baseline → runtime → AgentSpec → Turn.
use crate::{Access, AgentDefinitionSpec, DomainSet, Provider, ToolGroups};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Sampling and iteration defaults. `None` preserves the lower precedence value.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModelDefaults {
    /// Sampling temperature; unspecified preserves the lower precedence value.
    pub temperature: Option<f64>,
    /// Maximum output tokens; unspecified preserves the lower precedence value.
    pub max_tokens: Option<u32>,
    /// Nucleus sampling probability; unspecified preserves the lower precedence value.
    pub top_p: Option<f64>,
    /// Maximum model/tool iterations; unspecified preserves the lower precedence value.
    pub max_iterations: Option<usize>,
}
impl ModelDefaults {
    pub(crate) fn overlay(&self, other: &Self) -> Self {
        Self {
            temperature: other.temperature.or(self.temperature),
            max_tokens: other.max_tokens.or(self.max_tokens),
            top_p: other.top_p.or(self.top_p),
            max_iterations: other.max_iterations.or(self.max_iterations),
        }
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self
            .temperature
            .is_some_and(|v| !v.is_finite() || !(0.0..=2.0).contains(&v))
        {
            return Err("temperature must be finite and between 0 and 2".into());
        }
        if self
            .top_p
            .is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
        {
            return Err("top_p must be finite and between 0 and 1".into());
        }
        if self.max_tokens == Some(0) || self.max_iterations == Some(0) {
            return Err("model limits must be positive".into());
        }
        Ok(())
    }
}
/// Skill bundles copied into each agent and optional operator skill discovery.
#[derive(Debug, Clone, Default)]
pub struct SkillsPolicy {
    /// Directory containing skill bundles to copy into each agent.
    pub root: Option<PathBuf>,
    /// Whether operator skill roots are discovered.
    pub include_user_skills: bool,
}
/// Background memory learning policy, expressed using the existing memory recall settings.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LearningSettings {
    /// Turns between background belief builds; zero disables builds.
    pub build_beliefs_every: u32,
    /// Maximum learning entries included in recalled context.
    pub learnings_limit: u32,
}
impl Default for LearningSettings {
    fn default() -> Self {
        Self {
            build_beliefs_every: 10,
            learnings_limit: 8,
        }
    }
}
/// Effective defaults inherited by an agent which leaves its spec silent.
#[derive(Clone)]
pub struct AgentDefaults {
    /// Default inference provider and model route.
    pub provider: Provider,
    /// Default acting authority and approval policy.
    pub access: Access,
    /// Domain families available to new agents.
    pub domains: DomainSet,
    /// Default tool group disclosure ceiling.
    pub tool_groups: ToolGroups,
    /// Base definition extended by each agent.
    pub definition: AgentDefinitionSpec,
    /// Default shell and file confinement.
    pub sandbox: crate::SandboxModeSpec,
    /// Resolved sampling and iteration defaults.
    pub model: ModelDefaults,
    /// Skill bundle installation and user discovery policy.
    pub skills: SkillsPolicy,
    #[cfg(feature = "mcp")]
    /// MCP servers inherited by each agent.
    pub mcp_baseline: Vec<crate::McpServer>,
    /// Reusable named definition templates.
    pub templates: BTreeMap<String, AgentDefinitionSpec>,
}
impl Default for AgentDefaults {
    fn default() -> Self {
        Self {
            provider: Provider::inherit(),
            access: Access::default(),
            domains: super::presets::default_domains(),
            tool_groups: ToolGroups::default(),
            definition: AgentDefinitionSpec::default(),
            sandbox: crate::SandboxModeSpec::None,
            model: ModelDefaults::default(),
            skills: SkillsPolicy::default(),
            #[cfg(feature = "mcp")]
            mcp_baseline: Vec::new(),
            templates: BTreeMap::new(),
        }
    }
}
/// Runtime defaults snapshot. Agent defaults are resolved against the runtime config.
pub type RuntimeDefaults = AgentDefaults;

#[cfg(test)]
#[path = "defaults_tests.rs"]
mod tests;

//! Runtime module selection is independent from Cargo dependency selection.
use super::{RuntimeBuilder, RuntimeError};
use crate::{DomainSet, GroupMode, ServiceSet, ToolGroups};
/// Module families selectable as a coordinated domain/service/tool-group unit.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeModule {
    /// Agent definitions, orchestration and model/tool turns.
    Agent,
    /// Memory records and agent recall.
    Memory,
    /// Model routing and inference; does not require the microphone feature.
    Inference,
    /// Skill registry and execution.
    Skills,
    /// MCP server clients and tool discovery.
    Mcp,
    /// Workflow graph execution.
    Flows,
    /// Message channels and listeners.
    Channels,
    /// Speech input and output.
    Voice,
    /// Image and video generation.
    Media,
    /// Wallet and payment tools.
    Web3,
    /// External connector actions.
    Integrations,
    /// Scheduled jobs.
    Automation,
    /// Code execution runtimes.
    Runtimes,
    /// Loadable native module host.
    Modules,
    /// Office document extraction and generation tools.
    Documents,
    /// Hosting provider deployment tools.
    Hosting,
}
impl RuntimeModule {
    /// Cargo gate required by this module, when it has one.
    pub fn required_feature(self) -> Option<&'static str> {
        match self {
            Self::Skills => Some("skills"),
            Self::Mcp => Some("mcp"),
            Self::Flows => Some("flows"),
            Self::Channels => Some("channels"),
            Self::Voice => Some("voice"),
            Self::Media => Some("media"),
            Self::Web3 => Some("web3"),
            Self::Modules => Some("modules"),
            Self::Documents => Some("documents"),
            Self::Hosting => Some("hosting"),
            _ => None,
        }
    }
}
/// Approximate runtime activity, not a claim about compile-time dependency size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeightClass {
    /// Agent and memory only, without background services.
    Lean,
    /// Agent core with compiled skills, MCP and workflows.
    Standard,
    /// All compiled module families and their supporting services.
    Full,
    /// An explicit selection or manually configured builder.
    Custom,
}
#[derive(Clone)]
pub(crate) struct ModuleSelection {
    pub modules: Vec<RuntimeModule>,
    pub weight: WeightClass,
}
impl RuntimeBuilder {
    /// Agent and memory only; no background services or optional tool groups.
    /// Pair with `--no-default-features` to disable Cargo default feature gates.
    pub fn lean() -> Self {
        Self::new()
            .modules([
                RuntimeModule::Agent,
                RuntimeModule::Memory,
                RuntimeModule::Inference,
            ])
            .weight(WeightClass::Lean)
    }
    /// Agent core plus the compiled skills, MCP and workflow families. No background services.
    pub fn standard() -> Self {
        let features = openhuman_core::core::runtime::compiled_features();
        let mut modules = vec![
            RuntimeModule::Agent,
            RuntimeModule::Memory,
            RuntimeModule::Inference,
        ];
        for module in [
            RuntimeModule::Skills,
            RuntimeModule::Mcp,
            RuntimeModule::Flows,
        ] {
            if module.required_feature().is_some_and(|f| features[f]) {
                modules.push(module);
            }
        }
        Self::new().modules(modules).weight(WeightClass::Standard)
    }
    /// All available module families, with their supporting background services.
    pub fn full() -> Self {
        use RuntimeModule::*;
        let features = openhuman_core::core::runtime::compiled_features();
        let modules = [
            Agent,
            Memory,
            Inference,
            Skills,
            Mcp,
            Flows,
            Channels,
            Voice,
            Media,
            Web3,
            Integrations,
            Automation,
            Runtimes,
            Modules,
            Documents,
            Hosting,
        ]
        .into_iter()
        .filter(|m| m.required_feature().is_none_or(|f| features[f]));
        let mut builder = Self::new().modules(modules).weight(WeightClass::Full);
        let domains = builder.domains.unwrap();
        builder.services = Some(ServiceSet {
            cron: domains.automation,
            channels: domains.channels,
            integrations: domains.integrations,
            memory_queue: domains.memory,
            memory_sync: domains.memory,
            skill_catalog_refresh: domains.skills,
            mcp_boot: domains.mcp,
            ..ServiceSet::none()
        });
        builder
    }
    fn weight(mut self, weight: WeightClass) -> Self {
        if let Some(s) = &mut self.selection {
            s.weight = weight;
        }
        self
    }
    /// Select exactly these families; missing Cargo gates are rejected before boot.
    pub fn modules(mut self, modules: impl IntoIterator<Item = RuntimeModule>) -> Self {
        let mut modules: Vec<_> = modules.into_iter().collect();
        modules.sort();
        modules.dedup();
        let mut d = DomainSet::kernel();
        let mut g = ToolGroups::none();
        for module in &modules {
            match module {
                RuntimeModule::Agent => d.agent = true,
                RuntimeModule::Memory => d.memory = true,
                RuntimeModule::Inference => d.inference = true,
                RuntimeModule::Skills => {
                    d.skills = true;
                    g = g.with("skills", GroupMode::Withheld);
                }
                RuntimeModule::Mcp => {
                    d.mcp = true;
                    g = g.with("mcp", GroupMode::Withheld);
                }
                RuntimeModule::Flows => {
                    d.flows = true;
                    g = g.with("workflows", GroupMode::Withheld);
                }
                RuntimeModule::Channels => d.channels = true,
                RuntimeModule::Voice => {
                    d.voice = true;
                    g = g.with("audio", GroupMode::Withheld);
                }
                RuntimeModule::Media => {
                    d.media = true;
                    g = g.with("media", GroupMode::Withheld);
                }
                RuntimeModule::Web3 => {
                    d.web3 = true;
                    g = g.with("web3", GroupMode::Withheld);
                }
                RuntimeModule::Integrations => {
                    d.integrations = true;
                    g = g.with("composio", GroupMode::Withheld);
                }
                RuntimeModule::Automation => {
                    d.automation = true;
                    g = g.with("scheduling", GroupMode::Withheld);
                }
                RuntimeModule::Runtimes => {
                    d.runtimes = true;
                    g = g.with("coding", GroupMode::Withheld);
                }
                RuntimeModule::Modules => d.modules = true,
                RuntimeModule::Documents => {
                    d.modules = true;
                    d.platform = true;
                    g = g.with("documents", GroupMode::Withheld);
                }
                RuntimeModule::Hosting => d.platform = true,
            }
        }
        self.domains = Some(d);
        self.tool_groups = Some(g);
        self.services = Some(ServiceSet {
            skill_catalog_refresh: d.skills,
            mcp_boot: d.mcp,
            ..ServiceSet::none()
        });
        self.selection = Some(ModuleSelection {
            modules,
            weight: WeightClass::Custom,
        });
        self
    }
    pub(super) fn validate_modules(&self) -> Result<(), RuntimeError> {
        let features = openhuman_core::core::runtime::compiled_features();
        if let Some(selection) = &self.selection {
            for module in &selection.modules {
                if let Some(feature) = module.required_feature() {
                    if !features[feature] {
                        return Err(RuntimeError::MissingFeature {
                            module: *module,
                            feature,
                        });
                    }
                }
            }
        }
        Ok(())
    }
}

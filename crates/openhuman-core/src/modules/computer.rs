//! What the Computer settings page shows about the TinyComputer module: its
//! lifecycle state, the routes the host configured, and — when asked — what
//! the module itself reports through `Describe`.

use serde::Serialize;
use tinycomputer_bus::agent::{Capabilities, SurfaceAvailability};

use super::types::{ModuleState, ModuleStatus};
use crate::config::Config;

/// What the module reports about itself.
#[derive(Debug, Clone, Serialize)]
pub struct ComputerCapabilities {
    pub contract_version: (u32, u32),
    /// Whether this host can talk to that contract.
    pub compatible: bool,
    pub jev_configured: bool,
    pub planner_configured: bool,
    pub rescue_configured: bool,
    pub surfaces: Vec<SurfaceAvailability>,
}

impl From<Capabilities> for ComputerCapabilities {
    fn from(value: Capabilities) -> Self {
        Self {
            contract_version: value.contract_version,
            compatible: tinycomputer_bus::is_compatible(value.contract_version),
            jev_configured: value.jev_configured,
            planner_configured: value.planner_configured,
            rescue_configured: value.rescue_configured,
            surfaces: value.surfaces,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ComputerStatus {
    pub module: Option<ModuleStatus>,
    /// `[computer] decision_model`.
    pub decision_model: &'static str,
    /// Who pays for decisions: `hosted`, `direct_openrouter`, `open_jev`,
    /// `sage`, or `unavailable`.
    pub decision_route: &'static str,
    /// Where planner and rescue calls go: `hosted`, `direct_openrouter`, or
    /// `unavailable`.
    pub planner_route: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<ComputerCapabilities>,
    /// Why `Describe` could not be read, when it was asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

async fn planner_route(config: &Config) -> &'static str {
    match super::computer_config::module_config(config).await["planner"]["provider"].as_str() {
        Some("tiny_humans") => "hosted",
        Some("open_router") => "direct_openrouter",
        _ => "unavailable",
    }
}

async fn describe(config: &Config) -> Result<Capabilities, String> {
    let proxy = super::desktop::proxy(config).await?;
    proxy
        .call(tinycomputer_bus::agent::names::methods::DESCRIBE, ())
        .await
        .map_err(|error| format!("TinyComputer Describe failed: {error}"))
}

/// Report the module. `load` asks the module itself (loading it if needed);
/// otherwise it is only asked when already serving.
pub async fn status(config: &Config, load: bool) -> ComputerStatus {
    let module = || {
        super::ops::list(config)
            .into_iter()
            .find(|item| item.id == super::desktop::MODULE_ID)
    };
    let before = module();
    let ask = load
        || before
            .as_ref()
            .is_some_and(|m| m.state == ModuleState::Ready);
    let (capabilities, error) = if ask {
        match describe(config).await {
            Ok(capabilities) => (Some(capabilities.into()), None),
            Err(error) => {
                tracing::debug!(%error, "[computer] describe failed");
                (None, Some(error))
            }
        }
    } else {
        (None, None)
    };
    ComputerStatus {
        module: if ask { module() } else { before },
        decision_model: config.computer.decision_model.as_str(),
        decision_route: super::computer_config::billing_route(config).await,
        planner_route: planner_route(config).await,
        capabilities,
        error,
    }
}

#[cfg(test)]
#[path = "computer_tests.rs"]
mod tests;

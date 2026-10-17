//! [`ComponentSet`]: which tiny dependencies the config leaves switched on.
//!
//! Resolved from `[components]` and applied as `effective = host_choice ∩
//! components` across [`DomainSet`], [`ServiceSet`] and [`ToolGroups`]. It only
//! removes: a component that is on never enables what the host left out. A flag
//! change takes effect on the next core start.

use crate::config::schema::components::ComponentsConfig;
use crate::core::all::DomainGroup;
use crate::core::runtime::{DomainSet, ServiceSet};
use crate::core::StructuredRpcError;
use crate::tools::toolpacks::{GroupMode, ToolGroups};

pub use crate::config::schema::components::COMPONENT_NAMES;

/// The error code carried in the message of a disabled-component answer.
pub const COMPONENT_DISABLED: &str = "COMPONENT_DISABLED";

/// Resolved `[components]` switches. `Default` is everything on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentSet {
    pub flows: bool,
    pub tinyflows: bool,
    pub memes: bool,
    pub documents: bool,
    pub search: bool,
    pub mcp: bool,
    pub connectors: bool,
    pub hosting: bool,
    pub juice: bool,
    pub memory: bool,
    pub wallet: bool,
    pub computer: bool,
    pub sandbox: bool,
    pub channels: bool,
    pub skills: bool,
    pub voice: bool,
}

impl Default for ComponentSet {
    fn default() -> Self {
        Self::all()
    }
}

impl ComponentSet {
    /// Every component on.
    pub fn all() -> Self {
        Self::from_config(&ComponentsConfig::default())
    }

    /// Resolve from the persisted `[components]` table.
    pub fn from_config(c: &ComponentsConfig) -> Self {
        Self {
            flows: c.flows,
            tinyflows: c.tinyflows,
            memes: c.memes,
            documents: c.documents,
            search: c.search,
            mcp: c.mcp,
            connectors: c.connectors,
            hosting: c.hosting,
            juice: c.juice,
            memory: c.memory,
            wallet: c.wallet,
            computer: c.computer,
            sandbox: c.sandbox,
            channels: c.channels,
            skills: c.skills,
            voice: c.voice,
        }
    }

    /// Whether the component named `name` is on. Unknown names are on: this is
    /// a narrowing set, and a name it does not know cannot have been switched off.
    pub fn enabled(&self, name: &str) -> bool {
        self.to_config().get(name).unwrap_or(true)
    }

    fn to_config(self) -> ComponentsConfig {
        ComponentsConfig {
            flows: self.flows,
            tinyflows: self.tinyflows,
            memes: self.memes,
            documents: self.documents,
            search: self.search,
            mcp: self.mcp,
            connectors: self.connectors,
            hosting: self.hosting,
            juice: self.juice,
            memory: self.memory,
            wallet: self.wallet,
            computer: self.computer,
            sandbox: self.sandbox,
            channels: self.channels,
            skills: self.skills,
            voice: self.voice,
        }
    }

    /// `Ok` when `name` is on, otherwise the typed `COMPONENT_DISABLED:<name>` error.
    pub fn require(&self, name: &str) -> Result<(), StructuredRpcError> {
        if self.enabled(name) {
            Ok(())
        } else {
            Err(component_disabled(name))
        }
    }

    /// The component that owns a domain family, if any.
    pub fn component_for_group(group: DomainGroup) -> Option<&'static str> {
        Some(match group {
            DomainGroup::Flows => "flows",
            DomainGroup::Skills => "skills",
            DomainGroup::Mcp => "mcp",
            DomainGroup::Channels => "channels",
            DomainGroup::Web3 => "wallet",
            DomainGroup::Voice => "voice",
            DomainGroup::Memory => "memory",
            DomainGroup::Integrations => "connectors",
            _ => return None,
        })
    }

    /// `host ∩ components` for domain families.
    #[must_use]
    pub fn narrow_domains(&self, host: DomainSet) -> DomainSet {
        let mut d = host;
        d.flows &= self.flows;
        d.skills &= self.skills;
        d.mcp &= self.mcp;
        d.channels &= self.channels;
        d.web3 &= self.wallet;
        d.voice &= self.voice;
        d.memory &= self.memory;
        d.integrations &= self.connectors;
        d
    }

    /// `host ∩ components` for background services.
    #[must_use]
    pub fn narrow_services(&self, host: ServiceSet) -> ServiceSet {
        let mut s = host;
        s.channels &= self.channels;
        s.cron &= self.tinyflows;
        s.memory_queue &= self.memory;
        s.memory_sync &= self.memory;
        s.skill_catalog_refresh &= self.skills;
        s.mcp_boot &= self.mcp;
        s.integrations &= self.connectors;
        s
    }

    /// `host ∩ components` for tool groups: a disabled component's groups go
    /// `Off`; every other group keeps the host's mode.
    #[must_use]
    pub fn narrow_tool_groups(&self, host: ToolGroups) -> ToolGroups {
        let mut g = host;
        let off: [(bool, &[&str]); 6] = [
            (self.flows, &["workflows"]),
            (self.documents, &["documents"]),
            (self.mcp, &["mcp"]),
            (self.connectors, &["composio"]),
            (self.wallet, &["web3"]),
            (self.skills, &["skills"]),
        ];
        for (on, ids) in off {
            if !on {
                for id in ids {
                    g = g.with(id, GroupMode::Off);
                }
            }
        }
        g
    }
}

/// The typed error a disabled component answers with: message
/// `COMPONENT_DISABLED:<name>` and `data.kind = "component_disabled"`.
pub fn component_disabled(name: &str) -> StructuredRpcError {
    StructuredRpcError {
        message: format!("{COMPONENT_DISABLED}:{name}"),
        data: Some(serde_json::json!({
            "kind": "component_disabled",
            "component": name,
        })),
        expected_user_state: true,
    }
}

/// Refuse an RPC handler with the encoded `COMPONENT_DISABLED:<name>` error
/// when the config it loaded has `name` switched off. For handlers that load
/// their own config and so cannot read the ambient scope.
pub fn require_config(config: &crate::config::Config, name: &str) -> Result<(), String> {
    if config.components.get(name).unwrap_or(true) {
        Ok(())
    } else {
        log::debug!("[rpc][component-gate] refused, component '{name}' disabled");
        Err(component_disabled(name).encode())
    }
}

#[cfg(test)]
#[path = "components_tests.rs"]
mod tests;

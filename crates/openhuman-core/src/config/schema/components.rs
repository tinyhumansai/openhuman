//! `[components]` — which tiny dependencies this core uses.
//!
//! One `bool` per dependency, all on by default. The set only *narrows*: it is
//! resolved into a [`ComponentSet`](crate::core::runtime::ComponentSet) that is
//! intersected with whatever the host chose (`DomainSet`, `ServiceSet`,
//! `ToolGroups`), so a switch here can never enable something the host left
//! out. There are no Cargo gates behind these; a disabled component is still
//! compiled in, just unused.
//!
//! Changing a switch takes effect on the next core start; there is no hot
//! toggling. The kernel dependencies (tinyagents, tinytools, tinyinference,
//! tinybus) are not toggleable, and `[modules] enabled` stays the global kill
//! switch for loadable modules.
//!
//! Each switch also reads `OPENHUMAN_COMPONENT_<NAME>=0|1`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::defaults::default_true;

/// Names of every toggleable component, in declaration order.
pub const COMPONENT_NAMES: [&str; 16] = [
    "flows",
    "tinyflows",
    "memes",
    "documents",
    "search",
    "mcp",
    "connectors",
    "hosting",
    "juice",
    "memory",
    "wallet",
    "computer",
    "sandbox",
    "channels",
    "skills",
    "voice",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ComponentsConfig {
    /// Saved workflows (tinyflows).
    #[serde(default = "default_true")]
    pub flows: bool,
    /// Scheduled jobs and workflow authoring runtime (the tinyflows module:
    /// scheduler, job store, schedule maths). Off, `cron.*` answers
    /// `COMPONENT_DISABLED:tinyflows` and no scheduler runs.
    #[serde(default = "default_true")]
    pub tinyflows: bool,
    /// Meme remixing (tinymemes).
    #[serde(default = "default_true")]
    pub memes: bool,
    /// Document extraction and generation (tinydocs).
    #[serde(default = "default_true")]
    pub documents: bool,
    /// Web search (tinysearch).
    #[serde(default = "default_true")]
    pub search: bool,
    /// MCP client (tinymcp).
    #[serde(default = "default_true")]
    pub mcp: bool,
    /// OAuth connectors (tinyconnectors).
    #[serde(default = "default_true")]
    pub connectors: bool,
    /// Hosting providers (tinyhosts).
    #[serde(default = "default_true")]
    pub hosting: bool,
    /// Tool-output compression (tinyjuice).
    #[serde(default = "default_true")]
    pub juice: bool,
    /// Memory engine (tinymemory).
    #[serde(default = "default_true")]
    pub memory: bool,
    /// Wallet, web3 and x402 payments (tinywallet).
    #[serde(default = "default_true")]
    pub wallet: bool,
    /// Computer use (tinycomputer).
    #[serde(default = "default_true")]
    pub computer: bool,
    /// Isolated execution (tinybox).
    #[serde(default = "default_true")]
    pub sandbox: bool,
    /// Messaging channels (tinychannels).
    #[serde(default = "default_true")]
    pub channels: bool,
    /// Skill bundles (tinyskills).
    #[serde(default = "default_true")]
    pub skills: bool,
    /// Voice (tinyvoice).
    #[serde(default = "default_true")]
    pub voice: bool,
}

impl Default for ComponentsConfig {
    fn default() -> Self {
        Self {
            flows: true,
            tinyflows: true,
            memes: true,
            documents: true,
            search: true,
            mcp: true,
            connectors: true,
            hosting: true,
            juice: true,
            memory: true,
            wallet: true,
            computer: true,
            sandbox: true,
            channels: true,
            skills: true,
            voice: true,
        }
    }
}

impl ComponentsConfig {
    /// The switch named `name`, or `None` for an unknown component.
    pub fn get(&self, name: &str) -> Option<bool> {
        Some(match name {
            "flows" => self.flows,
            "tinyflows" => self.tinyflows,
            "memes" => self.memes,
            "documents" => self.documents,
            "search" => self.search,
            "mcp" => self.mcp,
            "connectors" => self.connectors,
            "hosting" => self.hosting,
            "juice" => self.juice,
            "memory" => self.memory,
            "wallet" => self.wallet,
            "computer" => self.computer,
            "sandbox" => self.sandbox,
            "channels" => self.channels,
            "skills" => self.skills,
            "voice" => self.voice,
            _ => return None,
        })
    }

    /// Set the switch named `name`; returns `false` for an unknown component.
    pub fn set(&mut self, name: &str, value: bool) -> bool {
        let slot = match name {
            "flows" => &mut self.flows,
            "tinyflows" => &mut self.tinyflows,
            "memes" => &mut self.memes,
            "documents" => &mut self.documents,
            "search" => &mut self.search,
            "mcp" => &mut self.mcp,
            "connectors" => &mut self.connectors,
            "hosting" => &mut self.hosting,
            "juice" => &mut self.juice,
            "memory" => &mut self.memory,
            "wallet" => &mut self.wallet,
            "computer" => &mut self.computer,
            "sandbox" => &mut self.sandbox,
            "channels" => &mut self.channels,
            "skills" => &mut self.skills,
            "voice" => &mut self.voice,
            _ => return false,
        };
        *slot = value;
        true
    }
}

#[cfg(test)]
#[path = "components_tests.rs"]
mod tests;

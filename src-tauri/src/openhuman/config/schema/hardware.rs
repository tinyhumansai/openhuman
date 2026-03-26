//! Hardware (wizard-driven) configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Hardware transport mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, JsonSchema)]
pub enum HardwareTransport {
    #[default]
    None,
    Native,
    Serial,
    Probe,
}

impl std::fmt::Display for HardwareTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Native => write!(f, "native"),
            Self::Serial => write!(f, "serial"),
            Self::Probe => write!(f, "probe"),
        }
    }
}

fn default_baud_rate() -> u32 {
    115_200
}

/// Wizard-driven hardware configuration for physical world interaction.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HardwareConfig {
    /// Whether hardware access is enabled
    #[serde(default)]
    pub enabled: bool,
    /// Transport mode
    #[serde(default)]
    pub transport: HardwareTransport,
    /// Serial port path (e.g. "/dev/ttyACM0")
    #[serde(default)]
    pub serial_port: Option<String>,
    /// Serial baud rate
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,
    /// Probe target chip (e.g. "STM32F401RE")
    #[serde(default)]
    pub probe_target: Option<String>,
    /// Enable workspace datasheet RAG (index PDF schematics for AI pin lookups)
    #[serde(default)]
    pub workspace_datasheets: bool,
}

impl HardwareConfig {
    /// Return the active transport mode.
    pub fn transport_mode(&self) -> HardwareTransport {
        self.transport.clone()
    }
}

impl Default for HardwareConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            transport: HardwareTransport::None,
            serial_port: None,
            baud_rate: default_baud_rate(),
            probe_target: None,
            workspace_datasheets: false,
        }
    }
}

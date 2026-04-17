//! Cost tracking configuration.
//!
//! Identity is loaded from OpenClaw markdown files in the workspace
//! (`IDENTITY.md`, `SOUL.md`, etc.) and needs no config surface.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CostConfig {
    /// Enable cost tracking (default: false)
    #[serde(default)]
    pub enabled: bool,

    /// Daily spending limit in USD (default: 10.00)
    #[serde(default = "default_daily_limit")]
    pub daily_limit_usd: f64,

    /// Monthly spending limit in USD (default: 100.00)
    #[serde(default = "default_monthly_limit")]
    pub monthly_limit_usd: f64,

    /// Warn when spending reaches this percentage of limit (default: 80)
    #[serde(default = "default_warn_percent")]
    pub warn_at_percent: u8,

    /// Allow requests to exceed budget with --override flag (default: false)
    #[serde(default)]
    pub allow_override: bool,

    /// Per-model pricing (USD per 1M tokens)
    #[serde(default)]
    pub prices: HashMap<String, ModelPricing>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ModelPricing {
    /// Input price per 1M tokens
    #[serde(default)]
    pub input: f64,

    /// Output price per 1M tokens
    #[serde(default)]
    pub output: f64,
}

fn default_daily_limit() -> f64 {
    10.0
}

fn default_monthly_limit() -> f64 {
    100.0
}

fn default_warn_percent() -> u8 {
    80
}

impl Default for CostConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            daily_limit_usd: default_daily_limit(),
            monthly_limit_usd: default_monthly_limit(),
            warn_at_percent: default_warn_percent(),
            allow_override: false,
            prices: get_default_pricing(),
        }
    }
}

/// Default pricing for popular models (USD per 1M tokens)
fn get_default_pricing() -> HashMap<String, ModelPricing> {
    use super::types::{MODEL_AGENTIC_V1, MODEL_CODING_V1, MODEL_REASONING_V1};

    let mut prices = HashMap::new();

    prices.insert(
        MODEL_REASONING_V1.into(),
        ModelPricing {
            input: 0.84,
            output: 2.52,
        },
    );
    prices.insert(
        MODEL_AGENTIC_V1.into(),
        ModelPricing {
            input: 0.45,
            output: 1.80,
        },
    );
    prices.insert(
        MODEL_CODING_V1.into(),
        ModelPricing {
            input: 0.90,
            output: 3.30,
        },
    );

    prices
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct PeripheralsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub boards: Vec<PeripheralBoardConfig>,
    #[serde(default)]
    pub datasheet_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PeripheralBoardConfig {
    pub board: String,
    #[serde(default = "default_peripheral_transport")]
    pub transport: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default = "default_peripheral_baud")]
    pub baud: u32,
}

fn default_peripheral_transport() -> String {
    "serial".into()
}

fn default_peripheral_baud() -> u32 {
    115_200
}

impl Default for PeripheralBoardConfig {
    fn default() -> Self {
        Self {
            board: String::new(),
            transport: default_peripheral_transport(),
            path: None,
            baud: default_peripheral_baud(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_config_defaults() {
        let c = CostConfig::default();
        assert!(!c.enabled);
        assert_eq!(c.daily_limit_usd, 10.0);
        assert_eq!(c.monthly_limit_usd, 100.0);
        assert_eq!(c.warn_at_percent, 80);
        assert!(!c.allow_override);
        assert!(!c.prices.is_empty());
    }

    #[test]
    fn cost_config_default_pricing_has_known_models() {
        let c = CostConfig::default();
        assert!(c.prices.len() >= 3);
    }

    #[test]
    fn cost_config_serde_roundtrip() {
        let c = CostConfig::default();
        let json = serde_json::to_string(&c).unwrap();
        let back: CostConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.daily_limit_usd, 10.0);
        assert_eq!(back.monthly_limit_usd, 100.0);
    }

    #[test]
    fn cost_config_toml_with_custom_values() {
        let toml = r#"
            enabled = true
            daily_limit_usd = 50.0
            monthly_limit_usd = 500.0
            warn_at_percent = 90
            allow_override = true
        "#;
        let c: CostConfig = toml::from_str(toml).unwrap();
        assert!(c.enabled);
        assert_eq!(c.daily_limit_usd, 50.0);
        assert_eq!(c.monthly_limit_usd, 500.0);
        assert_eq!(c.warn_at_percent, 90);
        assert!(c.allow_override);
    }

    #[test]
    fn model_pricing_defaults_to_zero() {
        let p: ModelPricing = serde_json::from_str("{}").unwrap();
        assert_eq!(p.input, 0.0);
        assert_eq!(p.output, 0.0);
    }

    #[test]
    fn peripherals_config_defaults() {
        let p = PeripheralsConfig::default();
        assert!(!p.enabled);
        assert!(p.boards.is_empty());
        assert!(p.datasheet_dir.is_none());
    }

    #[test]
    fn peripheral_board_config_defaults() {
        let b = PeripheralBoardConfig::default();
        assert_eq!(b.transport, "serial");
        assert_eq!(b.baud, 115_200);
        assert!(b.board.is_empty());
        assert!(b.path.is_none());
    }

    #[test]
    fn peripheral_board_config_toml() {
        let toml = r#"
            board = "esp32"
            transport = "usb"
            path = "/dev/ttyUSB0"
            baud = 9600
        "#;
        let b: PeripheralBoardConfig = toml::from_str(toml).unwrap();
        assert_eq!(b.board, "esp32");
        assert_eq!(b.transport, "usb");
        assert_eq!(b.path.as_deref(), Some("/dev/ttyUSB0"));
        assert_eq!(b.baud, 9600);
    }

    #[test]
    fn peripherals_config_serde_roundtrip() {
        let p = PeripheralsConfig {
            enabled: true,
            boards: vec![PeripheralBoardConfig {
                board: "arduino".into(),
                transport: "serial".into(),
                path: Some("/dev/cu.usbmodem".into()),
                baud: 115_200,
            }],
            datasheet_dir: Some("/tmp/sheets".into()),
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: PeripheralsConfig = serde_json::from_str(&json).unwrap();
        assert!(back.enabled);
        assert_eq!(back.boards.len(), 1);
        assert_eq!(back.boards[0].board, "arduino");
    }
}

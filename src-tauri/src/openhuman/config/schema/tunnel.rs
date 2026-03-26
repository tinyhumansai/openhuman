//! Tunnel (Cloudflare, Tailscale, ngrok, custom) configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TunnelConfig {
    pub provider: String,
    #[serde(default)]
    pub cloudflare: Option<CloudflareTunnelConfig>,
    #[serde(default)]
    pub tailscale: Option<TailscaleTunnelConfig>,
    #[serde(default)]
    pub ngrok: Option<NgrokTunnelConfig>,
    #[serde(default)]
    pub custom: Option<CustomTunnelConfig>,
}

impl Default for TunnelConfig {
    fn default() -> Self {
        Self {
            provider: "none".into(),
            cloudflare: None,
            tailscale: None,
            ngrok: None,
            custom: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CloudflareTunnelConfig {
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TailscaleTunnelConfig {
    #[serde(default)]
    pub funnel: bool,
    pub hostname: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NgrokTunnelConfig {
    pub auth_token: String,
    pub domain: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CustomTunnelConfig {
    pub start_command: String,
    pub health_url: Option<String>,
    pub url_pattern: Option<String>,
}

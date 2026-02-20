//! Gateway (pairing, webhook, rate limits) configuration.

use super::defaults;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GatewayConfig {
    #[serde(default = "default_gateway_port")]
    pub port: u16,
    #[serde(default = "default_gateway_host")]
    pub host: String,
    #[serde(default = "default_true")]
    pub require_pairing: bool,
    #[serde(default)]
    pub allow_public_bind: bool,
    #[serde(default)]
    pub paired_tokens: Vec<String>,
    #[serde(default = "default_pair_rate_limit")]
    pub pair_rate_limit_per_minute: u32,
    #[serde(default = "default_webhook_rate_limit")]
    pub webhook_rate_limit_per_minute: u32,
    #[serde(default)]
    pub trust_forwarded_headers: bool,
    #[serde(default = "default_gateway_rate_limit_max_keys")]
    pub rate_limit_max_keys: usize,
    #[serde(default = "default_idempotency_ttl_secs")]
    pub idempotency_ttl_secs: u64,
    #[serde(default = "default_gateway_idempotency_max_keys")]
    pub idempotency_max_keys: usize,
}

fn default_true() -> bool {
    defaults::default_true()
}

fn default_gateway_port() -> u16 {
    3000
}

fn default_gateway_host() -> String {
    "127.0.0.1".into()
}

fn default_pair_rate_limit() -> u32 {
    10
}

fn default_webhook_rate_limit() -> u32 {
    60
}

fn default_idempotency_ttl_secs() -> u64 {
    300
}

fn default_gateway_rate_limit_max_keys() -> usize {
    10_000
}

fn default_gateway_idempotency_max_keys() -> usize {
    10_000
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            port: default_gateway_port(),
            host: default_gateway_host(),
            require_pairing: true,
            allow_public_bind: false,
            paired_tokens: Vec::new(),
            pair_rate_limit_per_minute: default_pair_rate_limit(),
            webhook_rate_limit_per_minute: default_webhook_rate_limit(),
            trust_forwarded_headers: false,
            rate_limit_max_keys: default_gateway_rate_limit_max_keys(),
            idempotency_ttl_secs: default_idempotency_ttl_secs(),
            idempotency_max_keys: default_gateway_idempotency_max_keys(),
        }
    }
}

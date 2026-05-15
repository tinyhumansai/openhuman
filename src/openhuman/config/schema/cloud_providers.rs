//! Cloud provider credential schema.
//!
//! Each entry in `Config::cloud_providers` represents one configured LLM
//! backend (OpenHuman, OpenAI, Anthropic, OpenRouter, or a custom
//! OpenAI-compatible endpoint). The factory in
//! `crate::openhuman::providers::factory` resolves workload-to-provider
//! strings against this list at runtime.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Discriminator for a cloud provider entry.
///
/// Wire format is lowercase (e.g. `"openai"`). Dictates the default endpoint
/// and the auth header style used by the chat factory.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CloudProviderType {
    Openhuman,
    Openai,
    Anthropic,
    Openrouter,
    Custom,
}

impl CloudProviderType {
    /// Well-known default base URL for each provider type.
    pub fn default_endpoint(&self) -> &'static str {
        match self {
            Self::Openhuman => "https://api.openhuman.ai/v1",
            Self::Openai => "https://api.openai.com/v1",
            Self::Anthropic => "https://api.anthropic.com/v1",
            Self::Openrouter => "https://openrouter.ai/api/v1",
            Self::Custom => "",
        }
    }

    /// Human-readable label used in logs and error messages.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Openhuman => "OpenHuman",
            Self::Openai => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::Openrouter => "OpenRouter",
            Self::Custom => "Custom",
        }
    }

    /// Lowercase wire-format string (matches JSON serialisation).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Openhuman => "openhuman",
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::Openrouter => "openrouter",
            Self::Custom => "custom",
        }
    }
}

/// Endpoint config for one cloud LLM provider.
///
/// **Note on secrets**: API keys are NOT stored on this struct. They live in
/// `auth-profiles.json` via [`crate::openhuman::credentials::AuthService`],
/// encrypted at rest under the workspace's `.secret_key`. The factory looks
/// up the bearer token at call time by `provider_type.as_str()` (e.g.
/// `"openai"`, `"anthropic"`), mirroring how the Composio integration stores
/// its key under `"composio-direct:default"`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default)]
pub struct CloudProviderCreds {
    /// Opaque stable id, e.g. `"p_openai_a8c3f"`. Never shown in the UI.
    /// Generated once by [`generate_provider_id`] and never changes.
    pub id: String,
    /// Provider type determines default endpoint, the auth-profile lookup
    /// key, and the human-readable label.
    #[serde(rename = "type")]
    pub r#type: CloudProviderType,
    /// OpenAI-compatible base URL (`/v1/chat/completions` is appended).
    pub endpoint: String,
    /// Default model id sent to this provider when no per-workload override
    /// is configured.
    pub default_model: String,
}

impl Default for CloudProviderCreds {
    fn default() -> Self {
        Self {
            id: String::new(),
            r#type: CloudProviderType::Openhuman,
            endpoint: CloudProviderType::Openhuman.default_endpoint().to_string(),
            default_model: "reasoning-v1".to_string(),
        }
    }
}

/// Generate a short opaque id for a new provider entry.
///
/// Format: `"p_<type>_<5 random alphanumerics>"`, e.g. `"p_openai_a8c3f"`.
/// The random suffix is not cryptographically strong — it only needs to be
/// unique within a single user's config file.
pub fn generate_provider_id(t: &CloudProviderType) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    // Cheap pseudo-random from timestamp nanoseconds — adequate for local
    // config uniqueness without pulling in a PRNG crate.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let chars: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut suffix = String::with_capacity(5);
    let mut seed = nanos as usize;
    for _ in 0..5 {
        suffix.push(chars[seed % chars.len()] as char);
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        seed = (seed >> 33) ^ seed;
    }
    format!("p_{}_{}", t.as_str(), suffix)
}

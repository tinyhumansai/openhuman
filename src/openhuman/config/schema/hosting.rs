//! Hosting provider configuration (`[hosting]`).
//!
//! Where the credential for a hosting provider comes from, and whether the
//! `hosting_*` agent tools are offered at all. Read by
//! [`crate::openhuman::hosting::credentials`] when it resolves an account, and
//! by the tool registry when it decides whether to register the tools.
//!
//! The key may be left empty here: an empty key falls back to the provider's own
//! environment variables, which is what a self-hosted single-tenant deployment
//! wants. A multi-tenant host (OpenCompany) puts each user's key in this section
//! instead, and nothing else in the process can see it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct HostingConfig {
    /// Master switch. When `false`, no hosting tool is registered, whatever
    /// credentials exist.
    #[serde(default)]
    pub enabled: bool,

    /// The provider slug to act on — today, `vercel`.
    #[serde(default = "default_provider")]
    pub provider: String,

    /// The provider API key. Empty means "read it from the environment", which
    /// is [`tinyhosts::ProviderKind::credentials_from_env`]'s search order.
    #[serde(default)]
    pub api_key: String,

    /// The team, organization, or account scope to act as. Empty means the
    /// personal account.
    #[serde(default)]
    pub team: String,
}

impl std::fmt::Debug for HostingConfig {
    /// Redacts `api_key`. `Config` derives `Debug` and embeds `HostingConfig`,
    /// so a nested `{:?}` must never render the raw credential.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostingConfig")
            .field("enabled", &self.enabled)
            .field("provider", &self.provider)
            .field(
                "api_key",
                &if self.has_api_key() { "<redacted>" } else { "" },
            )
            .field("team", &self.team)
            .finish()
    }
}

fn default_provider() -> String {
    "vercel".to_string()
}

impl Default for HostingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: default_provider(),
            api_key: String::new(),
            team: String::new(),
        }
    }
}

impl HostingConfig {
    /// Whether a credential was configured here, rather than left to the
    /// environment.
    pub fn has_api_key(&self) -> bool {
        !self.api_key.trim().is_empty()
    }

    /// The configured team, or `None` when the field was left blank.
    pub fn team(&self) -> Option<&str> {
        let team = self.team.trim();
        (!team.is_empty()).then_some(team)
    }
}

#[cfg(test)]
#[path = "hosting_tests.rs"]
mod tests;

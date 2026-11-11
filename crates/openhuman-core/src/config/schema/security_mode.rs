//! Security mode configuration: whether the checks OpenHuman delegates to
//! TinySecurity run at all (`[security]`).
//!
//! With the default [`SecurityMode::Enforce`] every delegated check (the
//! prompt screen, action classification and approvals, path checks, the action
//! budget, egress privacy checks, redaction and the audit trail) asks the
//! module and fails closed whenever it cannot answer: not loaded, erroring or
//! slow. A host that runs its own controls, or that ships without the module,
//! turns them off with [`SecurityMode::Off`]; the checks then allow and
//! redaction passes content through unchanged. A build without the
//! `security-module` feature behaves as `off` whatever this says, because there
//! is no module to ask (`crate::security::switch`).
//!
//! This is not the daemon's [`SecurityConfig`](super::SecurityConfig), which
//! configures sandboxing, resource limits and the audit sink.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Whether the TinySecurity-delegated checks run.
///
/// Serializes as snake_case (`enforce` / `off`). A config missing the
/// `[security]` block, or its `mode` key, deserializes to
/// [`SecurityMode::Enforce`]. Any other value is a parse error.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SecurityMode {
    /// Ask TinySecurity, refusing whatever it cannot confirm.
    #[default]
    Enforce,
    /// Do not ask: every delegated check allows, and redaction passes content
    /// through. Implies `[prompt_screen] mode = "off"`.
    Off,
}

/// Whether TinySecurity's Jev judge may settle a call that needs approval.
///
/// Serializes as snake_case (`on` / `off`). A missing key is
/// [`JudgeMode::On`]. The judge then runs only when all of these hold:
/// - the host supplies a [`SecurityJudgeRoute`]. Stored credentials don't
///   count: they are read through TinySecurity, which takes the judge when it
///   loads (tinyhumansai/tinysecurity#54);
/// - security is enforced ([`SecurityMode::Enforce`]);
/// - `[privacy] mode` isn't `local_only`: the judge sends redacted call
///   details to its endpoint;
/// - the route's key is 4 to 4096 bytes and not blank, and its endpoint is at
///   most 4096 bytes: an absolute HTTPS URL, or HTTP on a literal loopback
///   address, with no credentials, query or fragment.
///
/// Otherwise every approval goes to a person. The judge only covers the
/// operator, copilot and sub-agent callers; scheduled and remote callers
/// always go to a person. It can only turn "needs approval" into a one-off
/// allow, never override a deny, and a failed or unsure judgement still goes
/// to a person.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JudgeMode {
    /// Judge approvals when the conditions above hold.
    #[default]
    On,
    /// Every approval goes to a person.
    Off,
}

/// The wire dialect a [`SecurityJudgeRoute`] speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityJudgeDialect {
    /// OpenRouter's, also spoken by the TinyHumans gateway.
    OpenRouter,
    /// OpenJev's.
    OpenJev,
}

/// Where the Jev judge is reached, and with which credential.
///
/// A host that holds its own Jev credential supplies one through its runtime
/// builder. Without one the judge stays off, whatever credentials are stored. Never
/// serialized, and its [`Debug`](std::fmt::Debug) hides the key.
#[derive(Clone, PartialEq, Eq)]
pub struct SecurityJudgeRoute {
    /// The credential sent to `endpoint`.
    pub api_key: String,
    /// The exact evaluation URL.
    pub endpoint: String,
    /// The wire dialect `endpoint` speaks.
    pub dialect: SecurityJudgeDialect,
}

impl SecurityJudgeRoute {
    /// Jev through OpenRouter.
    #[must_use]
    pub fn open_router(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            endpoint: "https://openrouter.ai/api/v1/systemone".to_owned(),
            dialect: SecurityJudgeDialect::OpenRouter,
        }
    }

    /// Jev through the TinyHumans gateway at `base_url`.
    #[must_use]
    pub fn tinyhumans(base_url: &str, api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            endpoint: format!(
                "{}/agent-integrations/openrouter/systemone",
                base_url.trim_end_matches('/')
            ),
            dialect: SecurityJudgeDialect::OpenRouter,
        }
    }

    /// The open Jev endpoint.
    #[must_use]
    pub fn open_jev(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            endpoint: "https://api.openjev.sh/v1/systemone".to_owned(),
            dialect: SecurityJudgeDialect::OpenJev,
        }
    }
}

impl std::fmt::Debug for SecurityJudgeRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecurityJudgeRoute([REDACTED])")
    }
}

/// The `[security]` config block.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SecurityModeConfig {
    /// Whether the checks run. Missing key → [`SecurityMode::Enforce`].
    pub mode: SecurityMode,
    /// Whether the Jev judge may settle approvals. Missing key →
    /// [`JudgeMode::On`].
    pub judge: JudgeMode,
}

impl SecurityModeConfig {
    /// The checks turned off: [`SecurityMode::Off`].
    #[must_use]
    pub fn off() -> Self {
        Self {
            mode: SecurityMode::Off,
            ..Self::default()
        }
    }

    /// Whether the checks run. False only when the config says
    /// [`SecurityMode::Off`].
    #[must_use]
    pub fn enforces(&self) -> bool {
        self.mode == SecurityMode::Enforce
    }
}

#[cfg(test)]
#[path = "security_mode_tests.rs"]
mod tests;

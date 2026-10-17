//! Config sub-facade — the first typed surface, and the proof of the pattern.
//!
//! Every other sub-facade (memory, workflows, chat, …) follows the
//! shape established here:
//!
//! 1. A borrowed newtype over `&Arc<CoreRuntime>` — zero-cost, no state.
//! 2. Facade-owned serde types, so hosts never name a domain's internal type
//!    and never touch `serde_json::Value`.
//! 3. Two-line methods delegating to `call`.
//!
//! Config is deliberately first because it is registered under
//! `DomainGroup::Platform`, which every preset enables — so a failure here is
//! unambiguously a facade bug rather than a gating question.
//!
//! # Host helpers
//!
//! Beside the [`Config`] sub-facade, this module carries the few free
//! functions a host needs *before* (or outside) a runtime: locating the
//! OpenHuman root, reading the active user marker, and loading the config the
//! way the core does. The core's full config type is [`RuntimeConfig`].

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::call::call;
use super::error::CoreError;
use openhuman_core::core::runtime::CoreRuntime;

/// Environment-driven runtime flags.
///
/// Mirrors the core's `RuntimeFlagsOut` (`config/ops/loader.rs`). Declared here
/// rather than re-exported so the facade's wire contract is explicit and a
/// change to the core's internal struct surfaces as a failing round-trip test
/// instead of silently altering the host-facing API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeFlags {
    /// `OPENHUMAN_BROWSER_ALLOW_ALL` — browser tools bypass the allowlist.
    pub browser_allow_all: bool,
    /// `OPENHUMAN_LOG_PROMPTS` — full prompts are written to the log.
    pub log_prompts: bool,
}

/// Typed access to persisted and environment configuration.
///
/// Obtained from [`Core::config`](super::Core::config); never constructed
/// directly.
pub struct Config<'a>(pub(super) &'a Arc<CoreRuntime>);

impl Config<'_> {
    /// Read the environment-driven runtime flags.
    ///
    /// Note this method always travels wrapped in the `{"result", "logs"}`
    /// envelope, because its handler emits a log unconditionally. That is
    /// handled in `call` and is invisible here — which is
    /// the entire point of routing every method through one helper.
    pub async fn runtime_flags(&self) -> Result<RuntimeFlags, CoreError> {
        call(
            self.0,
            "openhuman.config_get_runtime_flags",
            serde_json::json!({}),
        )
        .await
    }
}

/// Shared declarative tool-rule vocabulary used by the typed runtime builder.
pub use tinytools::{
    DefaultEffect, Patterns, RuleEffect, Surface, ToolMatcher, ToolRule, ToolRules,
};

/// Typed runtime builder policy groups from the core configuration contract.
pub use openhuman_core::config::schema::{
    AutonomyConfig, CronConfig, PrivacyConfig, PrivacyMode, SecretsConfig,
};
/// The core's config type, as [`crate::RuntimeConfig`].
pub use openhuman_core::config::Config as RuntimeConfig;

/// Load `config.toml` (creating defaults when absent) with the environment
/// overlay and per-user scoping — what a core with no supplied config boots
/// from.
pub async fn load_or_init() -> anyhow::Result<RuntimeConfig> {
    log::debug!("[embed][config] load_or_init");
    RuntimeConfig::load_or_init().await
}

/// [`load_or_init`] under the core's RPC load timeout, for a host entry that
/// runs outside any turn (a desktop command, gateway activation): it reads the
/// host's own context ([`DispatchCtx::host_default`]), honouring an
/// embedder-supplied config on it. The error is user-presentable.
///
/// [`DispatchCtx::host_default`]: openhuman_core::core::runtime::DispatchCtx::host_default
pub async fn load_host_config() -> Result<RuntimeConfig, String> {
    openhuman_core::config::ops::load_config_in(
        &openhuman_core::core::runtime::DispatchCtx::host_default(),
    )
    .await
}

/// The OpenHuman root (`~/.openhuman`, or `OPENHUMAN_WORKSPACE`'s root) that
/// holds `active_user.toml` and the per-user trees.
pub fn default_root_openhuman_dir() -> anyhow::Result<PathBuf> {
    openhuman_core::config::default_root_openhuman_dir()
}

/// The signed-in user id recorded under `root` (see
/// [`default_root_openhuman_dir`]), if any. Best-effort: an unreadable marker
/// reads as `None`.
pub fn read_active_user_id(root: &Path) -> Option<String> {
    openhuman_core::config::read_active_user_id(root)
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;

/// Operator search policy and direct-route credential vocabulary.
pub use openhuman_core::config::{
    PresentationMode, ProviderRoute, SearchConfig, SearchEngineCredentials, SearchProviderSettings,
};
/// The three native search capabilities whose names an operator can present.
pub use openhuman_core::profiles::provider_settings::SearchRole;

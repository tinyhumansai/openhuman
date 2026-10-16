//! Registered live voice providers, keyed by provider slug.
//!
//! The built-in catalogue in [`super::providers`] covers the providers the
//! core can reach itself. An outside crate registers further ones here: each
//! entry says how the provider is listed, whether it is usable right now, how
//! a session's [`LiveConfig`] is shaped for it, and how to connect it.
//! [`prepare`](super::providers::prepare) asks this registry first, so a
//! registered slug wins over a built-in one; with nothing registered the
//! built-in behaviour is unchanged.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tinyagents_live::tinyliveagents::LiveConfig;

use super::error::LiveVoiceError;
use super::providers::PreparedProvider;
use super::types::{LiveProviderInfo, LiveProviderKind};
use crate::config::Config;
use crate::core::slots::HookSlot;

/// A boxed `Send` future, the shape a registered `prepare` returns.
pub type LiveFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Whether a registered provider is usable right now for `config`.
pub type ConfiguredFn = Arc<dyn Fn(&Config) -> bool + Send + Sync>;
/// Shapes a session's [`LiveConfig`] for a registered provider
/// (`config`, the system instruction, the live config to adjust).
pub type ConfigureFn = Arc<dyn Fn(&Config, &str, &mut LiveConfig) + Send + Sync>;
/// Connects a registered provider.
pub type PrepareFn = Arc<
    dyn Fn(Config, LiveConfig) -> LiveFuture<Result<PreparedProvider, LiveVoiceError>>
        + Send
        + Sync,
>;

/// Where a registered provider is listed relative to the built-in ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LiveProviderPlacement {
    /// After every built-in provider (the default).
    #[default]
    AfterBuiltIn,
    /// Before every built-in provider.
    BeforeBuiltIn,
}

/// One registered live provider.
#[derive(Clone)]
pub struct LiveProviderEntry {
    /// Display label.
    pub label: String,
    /// Hosted or bring-your-own-key.
    pub kind: LiveProviderKind,
    /// Voices the provider offers.
    pub voices: Vec<String>,
    /// Languages the provider speaks.
    pub languages: Vec<String>,
    /// The `provider:<slug>` key the provider reads, if any.
    pub key_slug: Option<String>,
    /// Whether the provider can be used right now.
    pub configured: ConfiguredFn,
    /// Optional per-session [`LiveConfig`] shaping.
    pub configure: Option<ConfigureFn>,
    /// Connects the provider.
    pub prepare: PrepareFn,
    /// Where the provider is listed relative to the built-in ones. Providers
    /// sharing a placement keep registration order.
    pub placement: LiveProviderPlacement,
}

static LIVE_PROVIDERS: HookSlot<LiveProviderEntry> = HookSlot::new("voice::live::providers");

/// Registers the live provider `slug` for the whole process.
pub fn register_live_provider(slug: impl Into<String>, entry: LiveProviderEntry) {
    LIVE_PROVIDERS.register(slug, entry);
}

/// Registers the live provider `slug` for the context built from `extensions`.
pub fn register_live_provider_in(
    extensions: &crate::core::slots::Extensions,
    slug: impl Into<String>,
    entry: LiveProviderEntry,
) {
    LIVE_PROVIDERS.register_in(extensions, slug, entry);
}

/// Removes the process-wide live provider `slug`.
pub fn unregister_live_provider(slug: &str) -> bool {
    LIVE_PROVIDERS.unregister(slug)
}

/// Lists registered providers before or after the built-in ones according to
/// their [`LiveProviderPlacement`]; a registered slug that is also built in
/// replaces the built-in row in place.
pub(super) fn merge_provider_infos(infos: &mut Vec<LiveProviderInfo>, config: &Config) {
    let mut leading = Vec::new();
    for (slug, entry) in LIVE_PROVIDERS.entries() {
        let info = LiveProviderInfo {
            id: slug.clone(),
            label: entry.label.clone(),
            kind: entry.kind,
            configured: (entry.configured)(config),
            key_slug: entry.key_slug.clone(),
            voices: entry.voices.clone(),
            languages: entry.languages.clone(),
        };
        match infos.iter_mut().find(|existing| existing.id == slug) {
            Some(existing) => *existing = info,
            None => match entry.placement {
                LiveProviderPlacement::AfterBuiltIn => infos.push(info),
                LiveProviderPlacement::BeforeBuiltIn => leading.push(info),
            },
        }
    }
    if !leading.is_empty() {
        leading.append(infos);
        *infos = leading;
    }
}

/// Applies a registered provider's [`LiveConfig`] shaping, if it has any.
pub(super) fn configure(
    config: &Config,
    provider: &str,
    system_instruction: &str,
    live: &mut LiveConfig,
) {
    if let Some(configure) = LIVE_PROVIDERS
        .get(provider)
        .and_then(|entry| entry.configure.clone())
    {
        tracing::debug!(
            provider,
            "[voice-live] shaping config via registered provider"
        );
        configure(config, system_instruction, live);
    }
}

/// The registered entry for `provider`, if any.
pub(super) fn lookup(provider: &str) -> Option<LiveProviderEntry> {
    LIVE_PROVIDERS.get(provider)
}

/// Connects `provider` through its registered entry.
pub(super) async fn prepare_with(
    entry: &LiveProviderEntry,
    provider: &str,
    config: &Config,
    live: LiveConfig,
) -> Result<PreparedProvider, LiveVoiceError> {
    tracing::debug!(provider, "[voice-live] preparing via registered provider");
    (entry.prepare)(config.clone(), live).await
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;

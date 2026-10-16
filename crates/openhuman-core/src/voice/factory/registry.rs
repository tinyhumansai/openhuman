//! Registered speech providers, keyed by provider slug.
//!
//! [`create_stt_provider`](super::create_stt_provider) and
//! [`create_tts_provider`](super::create_tts_provider) ask these registries
//! first, so an outside crate can supply a provider under any slug (including
//! the built-in `"cloud"`, which it then replaces). A TTS provider returns the
//! same [`ReplySpeech`](tinyinference_voice::reply::ReplySpeech) envelope the
//! built-ins do, optional visemes included. With nothing registered the
//! hard-coded routes below apply unchanged.

use std::sync::Arc;

use super::traits::{SttProvider, TtsProvider};
use crate::config::Config;
use crate::core::slots::{Extensions, HookSlot};

/// Builds an STT provider: `(model, config)`; an empty `model` means "the
/// provider's default".
pub type SttProviderFactory =
    Arc<dyn Fn(&str, &Config) -> anyhow::Result<Box<dyn SttProvider>> + Send + Sync>;

/// Builds a TTS provider: `(voice, config)`; `None` means "the provider picks
/// its own default voice".
pub type TtsProviderFactory =
    Arc<dyn Fn(Option<&str>, &Config) -> anyhow::Result<Box<dyn TtsProvider>> + Send + Sync>;

static STT_PROVIDERS: HookSlot<SttProviderFactory> = HookSlot::new("voice::factory::stt");
static TTS_PROVIDERS: HookSlot<TtsProviderFactory> = HookSlot::new("voice::factory::tts");

/// Registers the STT provider `slug` for the whole process.
pub fn register_stt_provider<F>(slug: impl Into<String>, factory: F)
where
    F: Fn(&str, &Config) -> anyhow::Result<Box<dyn SttProvider>> + Send + Sync + 'static,
{
    STT_PROVIDERS.register(slug, Arc::new(factory));
}

/// Registers the STT provider `slug` for the context built from `extensions`.
pub fn register_stt_provider_in<F>(extensions: &Extensions, slug: impl Into<String>, factory: F)
where
    F: Fn(&str, &Config) -> anyhow::Result<Box<dyn SttProvider>> + Send + Sync + 'static,
{
    STT_PROVIDERS.register_in(extensions, slug, Arc::new(factory));
}

/// Removes the process-wide STT provider `slug`.
pub fn unregister_stt_provider(slug: &str) -> bool {
    STT_PROVIDERS.unregister(slug)
}

/// Registers the TTS provider `slug` for the whole process.
pub fn register_tts_provider<F>(slug: impl Into<String>, factory: F)
where
    F: Fn(Option<&str>, &Config) -> anyhow::Result<Box<dyn TtsProvider>> + Send + Sync + 'static,
{
    TTS_PROVIDERS.register(slug, Arc::new(factory));
}

/// Registers the TTS provider `slug` for the context built from `extensions`.
pub fn register_tts_provider_in<F>(extensions: &Extensions, slug: impl Into<String>, factory: F)
where
    F: Fn(Option<&str>, &Config) -> anyhow::Result<Box<dyn TtsProvider>> + Send + Sync + 'static,
{
    TTS_PROVIDERS.register_in(extensions, slug, Arc::new(factory));
}

/// Removes the process-wide TTS provider `slug`.
pub fn unregister_tts_provider(slug: &str) -> bool {
    TTS_PROVIDERS.unregister(slug)
}

pub(super) fn stt_factory(slug: &str) -> Option<SttProviderFactory> {
    STT_PROVIDERS.get(slug)
}

pub(super) fn tts_factory(slug: &str) -> Option<TtsProviderFactory> {
    TTS_PROVIDERS.get(slug)
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;

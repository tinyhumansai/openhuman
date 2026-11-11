//! Read-only readiness checks for STT and TTS configuration.

use std::sync::Arc;

use async_trait::async_trait;

use crate::config::Config;
use crate::doctor::{ComponentId, DoctorCheck, DoctorTier, Finding};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VoiceIssue {
    BuildDisabled,
    ProviderNotConfigured,
    ProviderUnsupported,
    CredentialMissing,
    PiperBinaryMissing,
    VoiceModelMissing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct VoiceSnapshot {
    pub stt: Result<(), VoiceIssue>,
    pub tts: Result<(), VoiceIssue>,
}

#[async_trait]
pub(super) trait VoiceStatusProbe: Send + Sync {
    async fn inspect(&self, config: &Config) -> VoiceSnapshot;
}

struct ConfigVoiceProbe;

#[async_trait]
impl VoiceStatusProbe for ConfigVoiceProbe {
    async fn inspect(&self, config: &Config) -> VoiceSnapshot {
        VoiceSnapshot {
            stt: stt_readiness(config).await,
            tts: tts_readiness(config).await,
        }
    }
}

pub(super) fn for_context() -> Vec<Arc<dyn DoctorCheck>> {
    checks_with_probe(Arc::new(ConfigVoiceProbe))
        .into_iter()
        .map(|check| Arc::new(check) as Arc<dyn DoctorCheck>)
        .collect()
}

fn checks_with_probe(probe: Arc<dyn VoiceStatusProbe>) -> Vec<VoiceCheck> {
    vec![
        VoiceCheck::configured(VoiceKind::Stt, Arc::clone(&probe)),
        VoiceCheck::configured(VoiceKind::Tts, probe),
    ]
}

#[derive(Clone, Copy)]
enum VoiceKind {
    Stt,
    Tts,
}

struct VoiceCheck {
    id: &'static str,
    kind: VoiceKind,
    probe: Arc<dyn VoiceStatusProbe>,
}

impl VoiceCheck {
    fn configured(kind: VoiceKind, probe: Arc<dyn VoiceStatusProbe>) -> Self {
        let (id, _) = kind.parts();
        Self { id, kind, probe }
    }

    async fn finding(&self, config: &Config) -> Finding {
        let snapshot = self.probe.inspect(config).await;
        let (id, result) = match self.kind {
            VoiceKind::Stt => (self.id, snapshot.stt),
            VoiceKind::Tts => (self.id, snapshot.tts),
        };
        match result {
            Ok(()) => Finding::pass(id, "The configured voice provider is ready."),
            Err(VoiceIssue::BuildDisabled) => Finding::skipped(
                id,
                "Voice support is not included in this build.",
                "Use a build that includes the voice component to inspect speech providers.",
            ),
            Err(issue) => Finding::error(id, issue_message(issue), issue_hint(issue)),
        }
    }
}

impl VoiceKind {
    fn parts(self) -> (&'static str, &'static str) {
        match self {
            Self::Stt => ("voice/stt", "speech-to-text"),
            Self::Tts => ("voice/tts", "text-to-speech"),
        }
    }
}

#[async_trait]
impl DoctorCheck for VoiceCheck {
    fn id(&self) -> &str {
        self.id
    }

    fn category(&self) -> &str {
        match self.kind {
            VoiceKind::Stt => "speech-to-text",
            VoiceKind::Tts => "text-to-speech",
        }
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Connect
    }

    fn component_gate(&self) -> Option<ComponentId> {
        Some(ComponentId::Voice)
    }

    async fn detect(&self, ctx: &crate::doctor::DoctorCtx) -> Vec<Finding> {
        vec![self.finding(ctx.config()).await]
    }
}

async fn stt_readiness(config: &Config) -> Result<(), VoiceIssue> {
    #[cfg(not(feature = "voice"))]
    {
        let _ = config;
        Err(VoiceIssue::BuildDisabled)
    }
    #[cfg(feature = "voice")]
    {
        let provider = crate::voice::effective_stt_provider(config);
        let slug = provider
            .split_once(':')
            .map_or(provider.as_str(), |(slug, _)| slug);
        if is_hosted_slug(slug) {
            return crate::voice::factory::registry::has_stt_provider(slug)
                .then_some(())
                .ok_or(VoiceIssue::ProviderNotConfigured);
        }
        let entry = config
            .voice_providers
            .iter()
            .find(|entry| entry.slug == slug)
            .ok_or(VoiceIssue::ProviderNotConfigured)?;
        if !entry.capability.supports_stt() {
            return Err(VoiceIssue::ProviderUnsupported);
        }
        provider_credential_present_read_only(config, slug)
            .await
            .then_some(())
            .ok_or(VoiceIssue::CredentialMissing)
    }
}

async fn tts_readiness(config: &Config) -> Result<(), VoiceIssue> {
    #[cfg(not(feature = "voice"))]
    {
        let _ = config;
        Err(VoiceIssue::BuildDisabled)
    }
    #[cfg(feature = "voice")]
    {
        let provider = crate::voice::factory::effective_tts_provider(config);
        let slug = provider
            .split_once(':')
            .map_or(provider.as_str(), |(slug, _)| slug);
        if slug == "piper" {
            let runtime = crate::inference::local_runtime_config(config);
            let binary =
                tinyinference_local::service::paths::resolve_piper_binary_with_config(&runtime);
            if binary.is_none() {
                return Err(VoiceIssue::PiperBinaryMissing);
            }
            return tinyinference_local::service::paths::resolve_tts_voice_path(&runtime)
                .map(|_| ())
                .map_err(|_| VoiceIssue::VoiceModelMissing);
        }
        if is_hosted_slug(slug) {
            return crate::voice::factory::registry::has_tts_provider(slug)
                .then_some(())
                .ok_or(VoiceIssue::ProviderNotConfigured);
        }
        let entry = config
            .voice_providers
            .iter()
            .find(|entry| entry.slug == slug)
            .ok_or(VoiceIssue::ProviderNotConfigured)?;
        if !entry.capability.supports_tts() {
            return Err(VoiceIssue::ProviderUnsupported);
        }
        provider_credential_present_read_only(config, slug)
            .await
            .then_some(())
            .ok_or(VoiceIssue::CredentialMissing)
    }
}

fn is_hosted_slug(slug: &str) -> bool {
    matches!(slug.trim(), "cloud" | "openhuman" | "backend")
}

async fn provider_credential_present_read_only(config: &Config, slug: &str) -> bool {
    use crate::credentials::{profiles::AuthProfileKind, AuthService};

    let key = crate::inference::provider::factory::auth_key_for_slug(slug);
    for provider in [key.as_str(), slug] {
        if let Ok(Some(profile)) = AuthService::get_profile_read_only(config, provider, None).await
        {
            let nonempty = match profile.kind {
                AuthProfileKind::Token => profile
                    .token
                    .as_deref()
                    .is_some_and(|token| !token.trim().is_empty()),
                AuthProfileKind::OAuth => profile
                    .token_set
                    .as_ref()
                    .is_some_and(|tokens| !tokens.access_token.trim().is_empty()),
            };
            if nonempty {
                return true;
            }
        }
    }

    if slug == "openai" {
        if let Ok(Some(profile)) = AuthService::get_profile_read_only(
            config,
            crate::credentials::openai_oauth::OPENAI_PROVIDER_KEY,
            Some(crate::credentials::openai_oauth::OPENAI_OAUTH_PROFILE_NAME),
        )
        .await
        {
            if let Some(tokens) = profile.token_set {
                return !tokens.access_token.trim().is_empty()
                    && !tokens.is_expiring_within(std::time::Duration::from_secs(120));
            }
        }
    }
    false
}

fn issue_message(issue: VoiceIssue) -> &'static str {
    match issue {
        VoiceIssue::BuildDisabled => "Voice support is not included in this build.",
        VoiceIssue::ProviderNotConfigured => {
            "No usable provider is registered for this voice route."
        }
        VoiceIssue::ProviderUnsupported => {
            "The selected voice provider does not support this operation."
        }
        VoiceIssue::CredentialMissing => "The selected voice provider has no saved credential.",
        VoiceIssue::PiperBinaryMissing => "The configured Piper executable was not found.",
        VoiceIssue::VoiceModelMissing => "The configured Piper voice model was not found.",
    }
}

fn issue_hint(issue: VoiceIssue) -> &'static str {
    match issue {
        VoiceIssue::BuildDisabled => "Use a build that includes the voice component.",
        VoiceIssue::ProviderNotConfigured | VoiceIssue::ProviderUnsupported => {
            "Select a supported speech provider in Voice settings."
        }
        VoiceIssue::CredentialMissing => {
            "Connect the selected speech provider or choose an available route."
        }
        VoiceIssue::PiperBinaryMissing => {
            "Install Piper through the normal setup path, then rerun doctor."
        }
        VoiceIssue::VoiceModelMissing => {
            "Install the selected Piper voice model, then rerun doctor."
        }
    }
}

#[cfg(test)]
#[path = "voice_tests.rs"]
mod tests;

//! Provider readiness checks over the selected search config.

use std::sync::Arc;

use async_trait::async_trait;

use crate::config::{Config, SEARCH_PROVIDERS};
use crate::doctor::{ComponentId, DoctorCheck, DoctorCtx, DoctorTier, Finding, FindingSeverity};
use crate::search::providers::{ProviderStatus, ResolvedProvider};

#[async_trait]
pub(super) trait SearchProviderResolver: Send + Sync {
    async fn resolve(&self, config: &Config) -> Vec<ResolvedProvider>;
}

struct ReadOnlyProviderResolver;

#[async_trait]
impl SearchProviderResolver for ReadOnlyProviderResolver {
    async fn resolve(&self, config: &Config) -> Vec<ResolvedProvider> {
        let effective = config
            .profile_providers
            .as_ref()
            .map(|binding| binding.effective_search(config))
            .unwrap_or_else(|| config.clone());
        let managed_available = managed_credential_available(&effective).await;
        crate::search::providers::resolve_with(&effective, managed_available)
    }
}

pub(super) fn for_context(ctx: &DoctorCtx) -> Vec<Arc<dyn DoctorCheck>> {
    let config = ctx.config();
    let effective = config
        .profile_providers
        .as_ref()
        .map(|binding| binding.effective_search(config))
        .unwrap_or_else(|| config.clone());
    effective
        .search
        .providers
        .keys()
        .filter(|id| SEARCH_PROVIDERS.contains(&id.as_str()))
        .map(|provider| {
            Arc::new(SearchProviderCheck::configured(
                provider,
                Arc::new(ReadOnlyProviderResolver),
            )) as Arc<dyn DoctorCheck>
        })
        .collect()
}

pub(super) struct SearchProviderCheck {
    id: String,
    provider: String,
    resolver: Arc<dyn SearchProviderResolver>,
}

impl SearchProviderCheck {
    fn configured(provider: &str, resolver: Arc<dyn SearchProviderResolver>) -> Self {
        Self {
            id: format!("search/{provider}"),
            provider: provider.to_owned(),
            resolver,
        }
    }

    pub(super) fn new(provider: &str) -> Self {
        Self::configured(provider, Arc::new(ReadOnlyProviderResolver))
    }

    #[cfg(test)]
    pub(super) fn with_resolver(provider: &str, resolver: Arc<dyn SearchProviderResolver>) -> Self {
        Self::configured(provider, resolver)
    }

    async fn finding(&self, config: &Config) -> Finding {
        let Some(provider) = self
            .resolver
            .resolve(config)
            .await
            .into_iter()
            .find(|provider| provider.id == self.provider)
        else {
            return Finding::error(
                &self.id,
                "The selected search provider is missing from the local provider catalog.",
                "Select a provider supported by this build and rerun doctor.",
            );
        };

        match provider.status(crate::search::is_active(config)) {
            ProviderStatus::Ready => Finding::pass(
                &self.id,
                format!("The {} search provider is ready.", self.provider),
            ),
            ProviderStatus::NeedsKey => Finding::new(
                &self.id,
                FindingSeverity::Warn,
                format!(
                    "The {} search provider needs a direct API key.",
                    self.provider
                ),
                format!("Set {} and rerun doctor.", direct_key_path(&self.provider)),
            ),
            ProviderStatus::SignInRequired => Finding::new(
                &self.id,
                FindingSeverity::Warn,
                format!(
                    "The managed {} search route needs an active backend credential.",
                    self.provider
                ),
                "Sign in to the selected backend, then rerun doctor.",
            ),
            ProviderStatus::Disabled => Finding::skipped(
                &self.id,
                format!(
                    "The {} search provider is disabled in the selected config.",
                    self.provider
                ),
                "Enable the provider in Search settings to inspect its readiness.",
            ),
            ProviderStatus::SearchOff => Finding::skipped(
                &self.id,
                "Search is disabled in the selected config.",
                "Enable Search in Components and rerun doctor.",
            ),
        }
    }
}

#[async_trait]
impl DoctorCheck for SearchProviderCheck {
    fn id(&self) -> &str {
        &self.id
    }

    fn category(&self) -> &str {
        "search"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Connect
    }

    fn component_gate(&self) -> Option<ComponentId> {
        Some(ComponentId::Search)
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        vec![self.finding(ctx.config()).await]
    }
}

fn direct_key_path(provider: &str) -> String {
    match provider {
        "searxng" => "searxng.base_url".into(),
        "seltz" => "seltz.api_key".into(),
        _ => format!("search.{provider}.api_key"),
    }
}

/// Resolve managed credential presence with read-only profile snapshots only.
/// This deliberately avoids `resolve_backend_credential`, which may publish a
/// session-expiry event, and never opens an auth-store writer or refreshes a
/// token.
async fn managed_credential_available(config: &Config) -> bool {
    use crate::credentials::{
        api_key,
        session_state::{
            check_session_backend, classify_session_token, is_local_session_token,
            SessionTokenCheck,
        },
        AuthService, APP_SESSION_PROVIDER,
    };

    if api_key::get_api_key_read_only(config)
        .await
        .ok()
        .flatten()
        .is_some_and(|key| !key.trim().is_empty())
    {
        return true;
    }
    let profile = AuthService::get_profile_read_only(config, APP_SESSION_PROVIDER, None)
        .await
        .ok()
        .flatten();
    if check_session_backend(config, profile.as_ref()).is_err() {
        return false;
    }
    matches!(
        classify_session_token(profile.as_ref(), chrono::Utc::now()),
        SessionTokenCheck::Live(token) if !is_local_session_token(&token)
    )
}

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;

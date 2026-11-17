//! Read-only, bounded provider catalog inspection for doctor.

use std::collections::HashMap;
use std::hash::{BuildHasher, Hash};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
use crate::core::{runtime::CoreContext, Outcome};
use crate::doctor::{
    contract::{DoctorCheck, DoctorCtx, DoctorTier, Finding, FindingSeverity},
    core::{ModelProbeEntry, ModelProbeOutcome, ModelProbeReport, ModelProbeSummary},
};
use crate::inference::registry::{
    inspection_catalog, InspectionCatalogError, InspectionCatalogRequest, PreparedInspectionCatalog,
};

const CACHE_TTL: Duration = Duration::from_secs(60);
const PROVIDER_CATALOG_TIMEOUT: Duration = Duration::from_secs(10);
type CachedCatalog = (
    Instant,
    Result<(serde_json::Value, Vec<String>), InspectionCatalogError>,
);
static CATALOG_CACHE: LazyLock<Mutex<HashMap<u64, CachedCatalog>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static CACHE_HASHER: LazyLock<std::collections::hash_map::RandomState> =
    LazyLock::new(std::collections::hash_map::RandomState::new);

fn hash_key<T: Hash>(value: &T) -> u64 {
    CACHE_HASHER.hash_one(value)
}

fn scope_key(config: &crate::config::Config) -> String {
    let context = CoreContext::scoped();
    let scoped_workspace = context
        .as_ref()
        .and_then(|context| context.workspace_dir().ok())
        .unwrap_or_else(|| config.workspace_dir.clone());
    let tenant = context
        .as_ref()
        .and_then(|context| context.profile())
        .unwrap_or_default();
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        config.config_path.display(),
        config.workspace_dir.display(),
        scoped_workspace.display(),
        tenant,
    )
}

fn configured_entries(config: &crate::config::Config) -> Vec<CloudProviderCreds> {
    let mut entries = config.cloud_providers.clone();
    let selected = selected_models(config);
    // The managed route is a configured model source even though it is not a
    // BYOK row in cloud_providers.
    if selected
        .iter()
        .any(|(_, model)| model.starts_with("openhuman:") || model.starts_with("openrouter/"))
        && !entries.iter().any(|entry| entry.slug == "openhuman")
    {
        entries.push(CloudProviderCreds {
            id: "openhuman".into(),
            slug: "openhuman".into(),
            label: "OpenHuman".into(),
            endpoint: config.api_url.clone().unwrap_or_default(),
            auth_style: AuthStyle::OpenhumanJwt,
            default_model: None,
        });
    }
    entries
}

async fn readonly_credential(
    config: &crate::config::Config,
    entry: &CloudProviderCreds,
) -> Option<String> {
    if entry.auth_style == AuthStyle::None || entry.auth_style == AuthStyle::OpenhumanJwt {
        return None;
    }
    let key = crate::inference::provider::factory::auth_key_for_slug(&entry.slug);
    let profile = crate::credentials::AuthService::get_profile_read_only(config, &key, None)
        .await
        .ok()??;
    let token = match profile.kind {
        crate::credentials::AuthProfileKind::Token => profile.token,
        crate::credentials::AuthProfileKind::OAuth => profile.token_set.and_then(|set| {
            (!set.is_expiring_within(Duration::from_secs(90))).then_some(set.access_token)
        }),
    };
    token.filter(|token| !token.trim().is_empty())
}

async fn prepare(
    config: &crate::config::Config,
    entry: &CloudProviderCreds,
) -> Result<PreparedInspectionCatalog, InspectionCatalogError> {
    if let Some(listing) = inspection_catalog(&entry.slug) {
        let client = crate::config::build_runtime_proxy_client_with_timeouts(
            "providers.list_models",
            30,
            10,
        );
        return listing(
            config.clone(),
            InspectionCatalogRequest {
                provider_key: String::new(),
                client,
            },
        )
        .await;
    }
    if entry.auth_style == AuthStyle::OpenhumanJwt {
        return Err(InspectionCatalogError::Unavailable);
    }
    let endpoint = entry.endpoint.trim_end_matches('/');
    let Ok(url) = reqwest::Url::parse(&format!("{endpoint}/models")) else {
        return Err(InspectionCatalogError::Unavailable);
    };
    if !matches!(url.scheme(), "http" | "https") {
        return Err(InspectionCatalogError::NoCatalog);
    }
    let token = readonly_credential(config, entry).await;
    let client = match config
        .cloud_provider_ca_certs
        .get(entry.slug.trim())
        .filter(|pem| !pem.is_empty())
    {
        Some(pem) => crate::util::tls::client_with_ca_bundle_with_timeouts(
            pem,
            "providers.list_models",
            30,
            10,
        )
        .map_err(|_| InspectionCatalogError::Unavailable)?,
        None => {
            crate::config::build_runtime_proxy_client_with_timeouts("providers.list_models", 30, 10)
        }
    };
    let provider = entry.slug.clone();
    let auth_style = entry.auth_style;
    let frozen_token = token.clone();
    let fetch_url = url.clone();
    let fetch = Box::pin(async move {
        let mut request = client.get(fetch_url);
        if let Some(token) = frozen_token.as_deref() {
            request = match auth_style {
                AuthStyle::Anthropic => request
                    .header("x-api-key", token)
                    .header("anthropic-version", "2023-06-01"),
                AuthStyle::Bearer => request.bearer_auth(token),
                AuthStyle::None | AuthStyle::OpenhumanJwt => request,
            };
        }
        let response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                InspectionCatalogError::Failure
            } else {
                InspectionCatalogError::Unavailable
            }
        })?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(InspectionCatalogError::NoCatalog);
        }
        if matches!(
            status,
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
        ) {
            return Err(InspectionCatalogError::AuthOrAccess);
        }
        if !status.is_success() {
            return Err(InspectionCatalogError::Failure);
        }
        let body = response
            .json::<serde_json::Value>()
            .await
            .map_err(|_| InspectionCatalogError::Failure)?;
        let models = tinyinference_llm::catalog::parse_models_response(&body)
            .map_err(|_| InspectionCatalogError::Failure)?;
        Ok(Outcome::new(
            serde_json::json!({"models": models}),
            Vec::new(),
        ))
    });
    Ok(PreparedInspectionCatalog::new(
        &provider,
        url.as_str(),
        token.as_deref(),
        fetch,
    ))
}

async fn catalog_for(
    config: &crate::config::Config,
    entry: &CloudProviderCreds,
    use_cache: bool,
) -> Result<(serde_json::Value, Vec<String>), InspectionCatalogError> {
    match tokio::time::timeout(
        PROVIDER_CATALOG_TIMEOUT,
        catalog_for_with_timeout(config, entry, use_cache),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(InspectionCatalogError::Failure),
    }
}

async fn catalog_for_with_timeout(
    config: &crate::config::Config,
    entry: &CloudProviderCreds,
    use_cache: bool,
) -> Result<(serde_json::Value, Vec<String>), InspectionCatalogError> {
    let prepared = prepare(config, entry).await?;
    let cache_key = hash_key(&(scope_key(config), entry.slug.clone(), prepared.identity()));
    if use_cache {
        if let Some((at, result)) = CATALOG_CACHE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&cache_key)
        {
            if at.elapsed() < CACHE_TTL {
                return result.clone();
            }
        }
    }
    let result = prepared
        .fetch()
        .await
        .map(|outcome| (outcome.value, outcome.logs));
    CATALOG_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(cache_key, (Instant::now(), result.clone()));
    result
}

pub async fn doctor_models(config: &crate::config::Config, use_cache: bool) -> ModelProbeReport {
    let mut entries = Vec::new();
    for provider in configured_entries(config) {
        let (outcome, message) = match catalog_for(config, &provider, use_cache).await {
            Ok((value, _)) => {
                let models = value
                    .get("models")
                    .and_then(serde_json::Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                (
                    ModelProbeOutcome::Ok,
                    Some(format!("Catalog returned {} models.", models.len())),
                )
            }
            Err(InspectionCatalogError::AuthOrAccess) => (
                ModelProbeOutcome::AuthOrAccess,
                Some("Catalog access was rejected.".into()),
            ),
            Err(InspectionCatalogError::NoCatalog) => (
                ModelProbeOutcome::Skipped,
                Some("Provider does not expose a models catalog.".into()),
            ),
            Err(InspectionCatalogError::Unavailable) => (
                ModelProbeOutcome::Skipped,
                Some("A safe read-only catalog is unavailable.".into()),
            ),
            Err(InspectionCatalogError::Failure) => (
                ModelProbeOutcome::Error,
                Some("Catalog request failed.".into()),
            ),
        };
        entries.push(ModelProbeEntry {
            provider: provider.slug,
            outcome,
            message,
        });
    }
    ModelProbeReport {
        summary: ModelProbeSummary {
            ok: entries
                .iter()
                .filter(|e| e.outcome == ModelProbeOutcome::Ok)
                .count(),
            skipped: entries
                .iter()
                .filter(|e| e.outcome == ModelProbeOutcome::Skipped)
                .count(),
            auth_or_access: entries
                .iter()
                .filter(|e| e.outcome == ModelProbeOutcome::AuthOrAccess)
                .count(),
            errors: entries
                .iter()
                .filter(|e| e.outcome == ModelProbeOutcome::Error)
                .count(),
        },
        entries,
    }
}

pub(crate) struct ModelInCatalog;
#[async_trait]
impl DoctorCheck for ModelInCatalog {
    fn id(&self) -> &str {
        "inference/model-in-catalog"
    }
    fn category(&self) -> &str {
        "inference"
    }
    fn tier(&self) -> DoctorTier {
        DoctorTier::Connect
    }
    fn is_blocking(&self) -> bool {
        false
    }
    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        let entries = configured_entries(ctx.config());
        if entries.is_empty() {
            return vec![Finding::skipped(
                self.id(),
                "No configured inference catalog is available.",
                "Configure a model provider and rerun doctor.",
            )];
        }
        let mut findings = Vec::new();
        for entry in entries {
            let catalog = match catalog_for(ctx.config(), &entry, true).await {
                Ok((value, _)) => value
                    .get("models")
                    .and_then(serde_json::Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
                Err(InspectionCatalogError::NoCatalog) => {
                    findings.push(Finding::skipped(
                        self.id(),
                        "Provider has no model catalog route.",
                        "Choose a provider with a read-only models route.",
                    ));
                    continue;
                }
                Err(InspectionCatalogError::Unavailable) => {
                    findings.push(Finding::skipped(
                        self.id(),
                        "Safe read-only model catalog is unavailable.",
                        "Reconnect the provider or use doctor.models for details.",
                    ));
                    continue;
                }
                Err(InspectionCatalogError::AuthOrAccess) => {
                    findings.push(Finding::new(
                        self.id(),
                        FindingSeverity::Warn,
                        "Provider rejected catalog access.",
                        "Check the selected provider credentials.",
                    ));
                    continue;
                }
                Err(InspectionCatalogError::Failure) => {
                    findings.push(Finding::new(
                        self.id(),
                        FindingSeverity::Warn,
                        "Provider catalog could not be read.",
                        "Check provider availability and rerun doctor.",
                    ));
                    continue;
                }
            };
            let ids = catalog
                .iter()
                .filter_map(|model| model.get("id").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>();
            let selected = selected_models(ctx.config());
            for (path, model) in selected
                .iter()
                .filter(|(_, model)| {
                    model.starts_with(&format!("{}:", entry.slug))
                        || (entry.slug == "openhuman" && model.starts_with("openrouter/"))
                })
                .map(|(path, model)| {
                    (
                        path.as_str(),
                        if model.starts_with("openrouter/") {
                            model.as_str()
                        } else {
                            model
                                .split_once(':')
                                .map(|(_, model)| model)
                                .unwrap_or(model.as_str())
                        },
                    )
                })
            {
                if !ids.contains(&model) {
                    let mut finding = Finding::new(
                        self.id(),
                        FindingSeverity::Warn,
                        format!(
                            "Configured model at {path} is not present in the provider catalog."
                        ),
                        "Choose a model listed by the provider or update this route.",
                    );
                    finding.config_path = Some(path.to_owned());
                    findings.push(finding);
                }
            }
        }
        findings
    }
}

fn selected_models(config: &crate::config::Config) -> Vec<(String, String)> {
    let mut models = Vec::new();
    if let Some(model) = &config.default_model {
        models.push(("default_model".to_owned(), model.clone()));
    }
    for (index, route) in config.model_routes.iter().enumerate() {
        models.push((format!("model_routes[{index}].model"), route.model.clone()));
    }
    models
}

pub(crate) struct AuthErrors;
#[async_trait]
impl DoctorCheck for AuthErrors {
    fn id(&self) -> &str {
        "inference/auth-errors"
    }
    fn category(&self) -> &str {
        "inference"
    }
    fn tier(&self) -> DoctorTier {
        DoctorTier::Connect
    }
    fn is_blocking(&self) -> bool {
        false
    }
    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        let mut findings = Vec::new();
        let (errors, has_unscoped) = crate::inference::auth_error_registry::snapshot_for_scope(
            &ctx.config().workspace_dir,
            &ctx.config().config_path,
            ctx.profile(),
        );
        for error in errors {
            findings.push(Finding::new(
                self.id(),
                FindingSeverity::Error,
                format!(
                    "Provider {} has a recorded authentication error.",
                    error.provider
                ),
                "Update or reconnect this provider's credentials.",
            ));
        }
        if has_unscoped {
            findings.push(Finding::new(
                self.id(),
                FindingSeverity::Info,
                "A provider authentication error was recorded outside a known inspection scope.",
                "Rerun doctor after reproducing the issue in the selected profile.",
            ));
        }
        findings
    }
}

pub(crate) fn built_in_checks() -> Vec<Arc<dyn DoctorCheck>> {
    vec![Arc::new(ModelInCatalog), Arc::new(AuthErrors)]
}

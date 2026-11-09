//! Bounded inspection calls to an already-ready connector runtime.

use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::config::Config;
use crate::modules::types::ModuleState;

const INSPECTION_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ConfiguredBinding {
    load_scope: [u8; 32],
    route_scope: [u8; 32],
    credential: [u8; 32],
}

fn loaded_scope() -> &'static Mutex<Option<[u8; 32]>> {
    static SCOPE: OnceLock<Mutex<Option<[u8; 32]>>> = OnceLock::new();
    SCOPE.get_or_init(|| Mutex::new(None))
}

fn pending_load_scope() -> &'static Mutex<Option<[u8; 32]>> {
    static SCOPE: OnceLock<Mutex<Option<[u8; 32]>>> = OnceLock::new();
    SCOPE.get_or_init(|| Mutex::new(None))
}

/// Safe result boundary for the existing-only connector call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExistingCallError {
    /// The runtime is absent or its accepted config belongs to another scope.
    Unavailable,
    /// A matching ready runtime returned a failed or malformed response.
    Failed,
}

trait ExistingRuntimeAccess {
    fn is_ready(&self) -> bool;
    fn proxy(&self) -> Result<tinybus::Proxy, ExistingCallError>;
}

struct ProcessRuntimeAccess;

impl ExistingRuntimeAccess for ProcessRuntimeAccess {
    fn is_ready(&self) -> bool {
        super::ops::state_of(super::MODULE_ID) == ModuleState::Ready
            && super::super::runtime::existing_runtime().is_some()
    }

    fn proxy(&self) -> Result<tinybus::Proxy, ExistingCallError> {
        let runtime =
            super::super::runtime::existing_runtime().ok_or(ExistingCallError::Unavailable)?;
        let record =
            super::registry::find(super::MODULE_ID).ok_or(ExistingCallError::Unavailable)?;
        runtime
            .proxy(record.bus_name, record.object_path)
            .map_err(|_| ExistingCallError::Unavailable)
    }
}

fn route_gate() -> &'static tokio::sync::Mutex<()> {
    static GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    &GATE
}

fn configured_binding() -> &'static Mutex<Option<ConfiguredBinding>> {
    static BINDING: OnceLock<Mutex<Option<ConfiguredBinding>>> = OnceLock::new();
    BINDING.get_or_init(|| Mutex::new(None))
}

pub(super) fn route_lock() -> &'static tokio::sync::Mutex<()> {
    route_gate()
}

/// Capture the scope only after the module accepted its CONFIGURE request.
/// The ordinary route fingerprint intentionally omits state_dir and toolkit
/// policy; this separate identity prevents those omissions from authorizing a
/// doctor read against a different workspace or configuration.
pub(super) async fn configured_binding_for_route(
    config: &Config,
    route: &serde_json::Value,
) -> Option<ConfiguredBinding> {
    let selected = selected_credential(config).await?;
    let accepted_secret = route
        .get("auth_token")
        .or_else(|| route.get("api_key"))?
        .as_str()?;
    if selected.1 != accepted_secret {
        return None;
    }
    let load_scope = loaded_scope()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .copied()?;
    Some(ConfiguredBinding {
        load_scope,
        route_scope: route_scope_fingerprint(config)?,
        credential: credential_fingerprint(&selected.0, &selected.1),
    })
}

pub(super) fn record_configured_route(binding: Option<ConfiguredBinding>) {
    *configured_binding()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = binding;
}

#[cfg(test)]
pub(super) async fn record_configured_route_for_test(config: &Config, route: &serde_json::Value) {
    let binding = async {
        let selected = selected_credential(config).await?;
        let accepted_secret = route
            .get("auth_token")
            .or_else(|| route.get("api_key"))?
            .as_str()?;
        if selected.1 != accepted_secret {
            return None;
        }
        let load_scope = loaded_scope()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .copied()?;
        Some(ConfiguredBinding {
            load_scope,
            route_scope: route_scope_fingerprint(config)?,
            credential: credential_fingerprint(&selected.0, &selected.1),
        })
    }
    .await;
    record_configured_route(binding);
}

/// Call a read-only member only when the currently serving module was already
/// configured for this exact selected scope and credential. This function
/// never starts the module runtime, loads the module, reconciles its route, or
/// imports legacy connector data. `None` means inspection is unavailable.
pub(crate) async fn call_existing_readonly<Request, Reply>(
    config: &Config,
    member: &str,
    request: Request,
) -> Result<Option<Reply>, ExistingCallError>
where
    Request: Serialize + Send,
    Reply: DeserializeOwned,
{
    let _route_guard = route_gate().lock().await;
    call_existing_readonly_with_runtime(config, member, request, &ProcessRuntimeAccess).await
}

async fn call_existing_readonly_with_runtime<Request, Reply, Runtime>(
    config: &Config,
    member: &str,
    request: Request,
    runtime: &Runtime,
) -> Result<Option<Reply>, ExistingCallError>
where
    Request: Serialize + Send,
    Reply: DeserializeOwned,
    Runtime: ExistingRuntimeAccess,
{
    if !config.modules.enabled || !config.components.connectors || !runtime.is_ready() {
        return Ok(None);
    }
    let Some(expected) = binding_for_config(config).await else {
        return Ok(None);
    };
    if *configured_binding()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        != Some(expected)
    {
        return Ok(None);
    }
    let proxy = runtime.proxy()?.with_timeout(INSPECTION_TIMEOUT);
    call_bound_proxy(config, &proxy, member, request).await
}

async fn call_bound_proxy<Request, Reply>(
    config: &Config,
    proxy: &tinybus::Proxy,
    member: &str,
    request: Request,
) -> Result<Option<Reply>, ExistingCallError>
where
    Request: Serialize + Send,
    Reply: DeserializeOwned,
{
    if !binding_matches_config(config).await {
        return Ok(None);
    }
    call_proxy_readonly(proxy, member, request).await.map(Some)
}

async fn call_proxy_readonly<Request, Reply>(
    proxy: &tinybus::Proxy,
    member: &str,
    request: Request,
) -> Result<Reply, ExistingCallError>
where
    Request: Serialize + Send,
    Reply: DeserializeOwned,
{
    call_proxy_readonly_with_timeout(proxy, member, request, INSPECTION_TIMEOUT).await
}

async fn call_proxy_readonly_with_timeout<Request, Reply>(
    proxy: &tinybus::Proxy,
    member: &str,
    request: Request,
    timeout: Duration,
) -> Result<Reply, ExistingCallError>
where
    Request: Serialize + Send,
    Reply: DeserializeOwned,
{
    match tokio::time::timeout(
        timeout,
        proxy
            .clone()
            .with_timeout(timeout)
            .call_confidential::<Reply>(member, (request,)),
    )
    .await
    {
        Ok(Ok(reply)) => Ok(reply),
        Ok(Err(_)) | Err(_) => Err(ExistingCallError::Failed),
    }
}

async fn selected_credential(config: &Config) -> Option<(String, String)> {
    if let Some(pinned) = config.composio.host_credential.as_ref() {
        return Some(("host-pinned-direct".to_owned(), pinned.api_key().to_owned()));
    }

    match config.composio.mode.trim() {
        "" | crate::config::schema::COMPOSIO_MODE_BACKEND => {
            let api_key = crate::credentials::api_key::get_api_key_read_only(config)
                .await
                .ok()?;
            if let Some(api_key) = api_key.filter(|key| !key.trim().is_empty()) {
                return Some(("backend-api-key".to_owned(), api_key.trim().to_owned()));
            }
            let profile =
                crate::credentials::session_state::load_app_session_profile_read_only(config)
                    .await
                    .ok()?;
            crate::credentials::session_state::check_session_backend(config, profile.as_ref())
                .ok()?;
            let token = match crate::credentials::session_state::classify_session_token(
                profile.as_ref(),
                chrono::Utc::now(),
            ) {
                crate::credentials::session_state::SessionTokenCheck::Live(token)
                    if !crate::credentials::session_state::is_local_session_token(token.trim()) =>
                {
                    token
                }
                _ => return None,
            };
            Some(("backend-session".to_owned(), token))
        }
        crate::config::schema::COMPOSIO_MODE_DIRECT => {
            let profile = crate::credentials::AuthService::get_profile_read_only(
                config,
                crate::integrations::composio::COMPOSIO_DIRECT_PROVIDER,
                None,
            )
            .await
            .ok()?;
            let stored = profile.and_then(|profile| match profile.kind {
                crate::credentials::profiles::AuthProfileKind::Token => profile.token,
                crate::credentials::profiles::AuthProfileKind::OAuth => {
                    profile.token_set.map(|tokens| tokens.access_token)
                }
            });
            stored
                .or_else(|| config.composio.api_key.clone())
                .map(|key| ("direct-composio-key".to_owned(), key.trim().to_owned()))
                .filter(|(_, key)| !key.is_empty())
        }
        _ => None,
    }
}

async fn binding_for_config(config: &Config) -> Option<ConfiguredBinding> {
    let actual_load_scope = loaded_scope()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .copied()?;
    if Some(actual_load_scope) != load_scope_fingerprint(config) {
        return None;
    }
    let selected = selected_credential(config).await?;
    binding_for_selected(config, &selected)
}

fn binding_for_selected(config: &Config, selected: &(String, String)) -> Option<ConfiguredBinding> {
    Some(ConfiguredBinding {
        load_scope: loaded_scope()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .copied()?,
        route_scope: route_scope_fingerprint(config)?,
        credential: credential_fingerprint(&selected.0, &selected.1),
    })
}

/// Capture the scope the resolver passed to the module's load configuration.
/// Search-path modules do not invoke this provider and therefore remain
/// unscoped for inspection rather than inheriting the first caller's config.
pub(super) fn note_module_config_for_load(config: &Config, module_config: &serde_json::Value) {
    let expected_state_dir = serde_json::to_value(config.workspace_dir.join("state")).ok();
    let expected_policy = super::offered_toolkits(config);
    let captures_selected_scope = module_config.get("state_dir") == expected_state_dir.as_ref()
        && module_config.get("offered_toolkits") == Some(&expected_policy);
    let candidate = captures_selected_scope
        .then(|| load_scope_fingerprint(config))
        .flatten();
    *pending_load_scope()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = candidate;
}

/// Promote the resolver's captured candidate only after ensure_loaded returned
/// success and the process-wide resolution table reports Ready.
pub(super) fn note_load_resolution_ready() {
    if super::ops::state_of(super::MODULE_ID) != ModuleState::Ready {
        return;
    }
    let candidate = pending_load_scope()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some(candidate) = candidate {
        *loaded_scope()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(candidate);
    }
}

async fn binding_matches_config(config: &Config) -> bool {
    let Some(expected) = binding_for_config(config).await else {
        return false;
    };
    *configured_binding()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        == Some(expected)
}

fn load_scope_fingerprint(config: &Config) -> Option<[u8; 32]> {
    let scope = serde_json::json!({
        "config_path": config.config_path,
        "workspace_dir": config.workspace_dir,
        "state_dir": config.workspace_dir.join("state"),
        "components_connectors": config.components.connectors,
        "modules_enabled": config.modules.enabled,
        "offered_toolkits": super::offered_toolkits(config),
    });
    Some(Sha256::digest(serde_json::to_vec(&scope).ok()?).into())
}

#[cfg(test)]
fn seed_loaded_scope_for_test(config: &Config) {
    *loaded_scope()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = load_scope_fingerprint(config);
}

fn route_scope_fingerprint(config: &Config) -> Option<[u8; 32]> {
    let config_json = serde_json::to_value(config).ok()?;
    let env = [
        "BACKEND_URL",
        "VITE_BACKEND_URL",
        "OPENHUMAN_APP_ENV",
        "VITE_APP_ENV",
        "OPENHUMAN_COMPOSIO_DIRECT_BASE_V2",
        "OPENHUMAN_COMPOSIO_DIRECT_BASE_V3",
        "TZ",
    ]
    .into_iter()
    .map(|key| (key, std::env::var(key).ok()))
    .collect::<std::collections::BTreeMap<_, _>>();
    let context = serde_json::json!({
        "config": config_json,
        "config_path": config.config_path,
        "workspace_dir": config.workspace_dir,
        "state_dir": config.workspace_dir.join("state"),
        "offered_toolkits": super::offered_toolkits(config),
        "host_credential": config.composio.host_credential.as_ref().map(|credential| {
            serde_json::json!({
                "entity_id": credential.entity(),
                "base_urls": credential.direct_base_urls().map(|urls| [&urls.v2, &urls.v3]),
            })
        }),
        "env": env,
    });
    Some(Sha256::digest(serde_json::to_vec(&context).ok()?).into())
}

fn credential_fingerprint(kind: &str, secret: &str) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(kind.as_bytes());
    hash.update([0]);
    hash.update(secret.as_bytes());
    hash.finalize().into()
}

#[cfg(test)]
#[path = "inspection_tests.rs"]
mod tests;

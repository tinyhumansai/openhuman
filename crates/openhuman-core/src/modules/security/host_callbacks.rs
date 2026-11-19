//! Authenticated callback bindings retained for the admitted TinySecurity peer.

use std::path::{Path, PathBuf};

use crate::config::Config;
use tinybus::module::{HostCallbackBinding, HostCallbackEndpoint, Resolver};

/// The admitted TinySecurity peer and the callback objects served on its
/// binding. Every tenant's secret registration is routed through these.
struct SecurityCallbackState {
    binding: std::sync::Arc<HostCallbackBinding>,
    consumer: std::sync::Arc<super::cipher_consumer::Consumer>,
    backends: std::sync::Arc<super::secret_backend::SecretBackends>,
}

type SecurityCallbackLoads =
    tokio::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<SecurityCallbackState>>>;

/// Admitted modules. Held across every tenant registration and retirement,
/// so the two never interleave for one approval scope.
static SECURITY_CALLBACK_LOADS: std::sync::LazyLock<SecurityCallbackLoads> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(std::collections::HashMap::new()));

pub(in crate::modules) use super::secret_tenants::Tenant;

fn secret_key_path(config_dir: &Path) -> PathBuf {
    config_dir.join(".secret_key")
}

fn config_key_path(config: &Config) -> PathBuf {
    secret_key_path(
        config
            .config_path
            .parent()
            .unwrap_or_else(|| Path::new(".")),
    )
}

/// Declare the config directory `config_dir` transient. See
/// [`crate::security::declare_transient_secret_scope`].
pub(crate) fn declare_transient_scope(config_dir: &Path) {
    super::secret_tenants::declare_transient(secret_key_path(config_dir));
}

/// Retire the secret registrations of every declared transient scope and
/// forget the declarations. See [`crate::security::release_secret_scope`].
pub(crate) fn release_scopes() {
    super::secret_tenants::release_transient();
}

/// Retire the secret registration of the config directory `config_dir` (a
/// closed or evicted profile). See [`crate::security::retire_secret_scope`].
pub(crate) fn retire_scope(config_dir: &Path) {
    super::secret_tenants::retire(&secret_key_path(config_dir));
}

pub(in crate::modules) async fn binding() -> Option<std::sync::Arc<HostCallbackBinding>> {
    SECURITY_CALLBACK_LOADS
        .lock()
        .await
        .get(super::MODULE_ID)
        .map(|state| std::sync::Arc::clone(&state.binding))
}

pub(in crate::modules) async fn cipher_consumer(
) -> Option<std::sync::Arc<super::cipher_consumer::Consumer>> {
    SECURITY_CALLBACK_LOADS
        .lock()
        .await
        .get(super::MODULE_ID)
        .map(|state| std::sync::Arc::clone(&state.consumer))
}

/// The live secret registration of `config`'s secret scope, once
/// [`ensure_loaded`] registered it.
pub(in crate::modules) fn tenant(config: &Config) -> Option<std::sync::Arc<Tenant>> {
    super::secret_tenants::live(&config_key_path(config))
}

/// The live tenant that owns secret registration `registration_id`.
pub(in crate::modules) fn tenant_by_registration(
    registration_id: &str,
) -> Option<std::sync::Arc<Tenant>> {
    super::secret_tenants::by_registration(registration_id)
}

pub(in crate::modules) async fn identity() -> Option<String> {
    SECURITY_CALLBACK_LOADS
        .lock()
        .await
        .get(super::MODULE_ID)
        .map(|state| state.binding.unique_name().to_string())
}

/// The neutral module configuration served to the admitted peer, both at
/// first load and on reinitialization.
pub(in crate::modules) async fn module_config(
    config: &Config,
) -> Result<serde_json::Value, String> {
    let judge = super::judge::resolve(config);
    let loads = SECURITY_CALLBACK_LOADS.lock().await;
    let state = loads
        .get(super::MODULE_ID)
        .ok_or_else(|| "security callback binding unavailable".to_owned())?;
    neutral_config(config, &state.binding, judge)
}

fn neutral_config(
    config: &Config,
    binding: &HostCallbackBinding,
    judge: Option<tinysecurity_bus::JudgeConfig>,
) -> Result<serde_json::Value, String> {
    let config = super::module_config_for_secret_authority(
        config,
        Some(binding.unique_name().to_string()),
        judge,
    )
    .map_err(str::to_owned)?;
    serde_json::to_value(config).map_err(|_| "invalid security module configuration".into())
}

fn callback_endpoints() -> Result<Vec<HostCallbackEndpoint>, String> {
    let path = tinybus::ObjectPath::new(tinysecurity_bus::callback_names2::OBJECT_PATH)
        .map_err(|_| "invalid security callback path".to_owned())?;
    let endpoint = |interface: &str, members: &[&str]| -> Result<HostCallbackEndpoint, String> {
        Ok(HostCallbackEndpoint {
            path: path.clone(),
            interface: interface
                .parse()
                .map_err(|_| "invalid security callback interface".to_owned())?,
            members: members
                .iter()
                .map(|member| {
                    (*member)
                        .parse()
                        .map_err(|_| "invalid security callback member".to_owned())
                })
                .collect::<Result<_, _>>()?,
        })
    };
    Ok(vec![
        endpoint(
            tinysecurity_bus::callback_names2::APPROVAL_STORE,
            &[
                tinysecurity_bus::callback_names2::LOAD,
                tinysecurity_bus::callback_names2::CAS,
            ],
        )?,
        endpoint(
            tinysecurity_bus::callback_names2::AUDIT_SINK,
            &[tinysecurity_bus::callback_names2::APPEND],
        )?,
        endpoint(
            tinysecurity_bus::callback_names2::APPROVAL_PROMPT,
            &[
                tinysecurity_bus::callback_names2::PROMPT,
                tinysecurity_bus::callback_names2::VERIFY_HUMAN,
            ],
        )?,
        endpoint(
            tinysecurity_bus::callback_names2::KEYRING_CONSENT,
            &[tinysecurity_bus::callback_names2::CONSENT],
        )?,
        endpoint(
            tinysecurity_bus::callback_names2::SECRET_CIPHER_CONSUMER,
            &[tinysecurity_bus::callback_names2::DELIVER],
        )?,
        endpoint(
            tinysecurity_bus::callback_names2::SECRET_BACKEND,
            &[
                tinysecurity_bus::callback_names2::LOAD,
                tinysecurity_bus::callback_names2::SET,
            ],
        )?,
        endpoint(
            tinysecurity_bus::callback_names2::DEVICE_PAIRING_HOST,
            &[
                tinysecurity_bus::callback_names2::RESOLVE_PAIRING_EVENT,
                tinysecurity_bus::callback_names2::PERSIST_DEVICE_PAIRING,
                tinysecurity_bus::callback_names2::LOAD_LEGACY_DEVICE_IDENTITIES,
            ],
        )?,
    ])
}

pub(super) fn opaque_id(label: &str, path: &Path, suffix: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(label.as_bytes());
    hasher.update((path.as_os_str().len() as u64).to_be_bytes());
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update(suffix.as_bytes());
    format!("{:x}", hasher.finalize())[..32].to_owned()
}

/// Admit module `id` under the neutral secret authority if it is not loaded
/// yet, then make sure `config`'s secret scope has its own live runtime
/// registration on it.
pub(in crate::modules) async fn ensure_loaded(
    resolver: &Resolver,
    config: &Config,
    id: &str,
) -> Result<(), String> {
    let mut loads = SECURITY_CALLBACK_LOADS.lock().await;
    if let Some(state) = loads.get(id) {
        let state = std::sync::Arc::clone(state);
        // Confirm the admitted module still retains this binding. Re-presenting
        // it would be refused (a binding is spent once a module takes it), and
        // the single-binding path accepts only a local-file source, so it
        // failed every later call on a host served from the release.
        super::super::ops::callback_load::confirm(resolver, id, &state.binding).await?;
        return ensure_tenant(config, &state).await;
    }

    let runtime = resolver
        .runtime()
        .await
        .map_err(|error| error.to_string())?;
    let host = runtime.host();
    let candidates = std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::<
        String,
        std::sync::Arc<SecurityCallbackState>,
    >::new()));
    let factory_config = config.clone();
    let factory_candidates = std::sync::Arc::clone(&candidates);

    let binding = resolver
        .ensure_loaded_with_host_callback_factory(id, move |_attempt| {
            let host = host;
            let config = factory_config.clone();
            let candidates = std::sync::Arc::clone(&factory_candidates);
            async move {
                let binding = std::sync::Arc::new(
                    host.prepare_host_callbacks(callback_endpoints()?)
                        .await
                        .map_err(|_| "security callback binding unavailable".to_owned())?,
                );
                let consumer = std::sync::Arc::new(super::cipher_consumer::Consumer::empty());
                let backends = std::sync::Arc::new(super::secret_backend::SecretBackends::empty());
                super::serve_secret_callbacks(
                    &binding,
                    std::sync::Arc::clone(&consumer),
                    std::sync::Arc::clone(&backends),
                )
                .await
                .map_err(|_| "security callback registration failed".to_owned())?;
                let judge = super::judge::resolve(&config);
                let module_config = neutral_config(&config, &binding, judge)?;
                let state = std::sync::Arc::new(SecurityCallbackState {
                    binding: std::sync::Arc::clone(&binding),
                    consumer,
                    backends,
                });
                candidates
                    .lock()
                    .await
                    .insert(binding.unique_name().to_string(), state);
                Ok((binding, module_config))
            }
        })
        .await?;

    let selected_key = binding.unique_name().to_string();
    let mut candidates = candidates.lock().await;
    let state = candidates
        .remove(&selected_key)
        .ok_or_else(|| "admitted security callback state unavailable".to_owned())?;
    candidates.clear();
    loads.insert(id.to_owned(), std::sync::Arc::clone(&state));
    ensure_tenant(config, &state).await
}

fn module_proxy(
    state: &SecurityCallbackState,
) -> Result<tinybus::module::HostCallbackProxy, String> {
    let record = super::super::registry::find(super::MODULE_ID)
        .ok_or_else(|| "security module registry record missing".to_owned())?;
    state
        .binding
        .proxy(
            record.bus_name,
            record.object_path,
            tinysecurity_bus::names::INTERFACE,
        )
        .map_err(|_| "security module proxy unavailable".to_owned())
}

/// Retire whatever was released, then return `config`'s live registration or
/// register a new one. The caller holds the loads lock.
///
/// Boxed as a `Send` future: it nests the whole registration, and leaving
/// it inline overflows the auto-trait evaluation of the callers' futures.
fn ensure_tenant<'a>(
    config: &'a Config,
    state: &'a SecurityCallbackState,
) -> futures::future::BoxFuture<'a, Result<(), String>> {
    Box::pin(ensure_tenant_inner(config, state))
}

async fn ensure_tenant_inner(config: &Config, state: &SecurityCallbackState) -> Result<(), String> {
    let proxy = module_proxy(state)?;
    let key_path = config_key_path(config);
    for tenant in super::secret_tenants::take_retiring() {
        if let Err(error) = retire_tenant(&proxy, state, &tenant).await {
            log::warn!("[security] secret registration retirement failed; kept for retry: {error}");
            let blocks_this_scope = tenant.key_path == key_path;
            super::secret_tenants::requeue(tenant);
            if blocks_this_scope {
                // The old registration may still be live under the approval
                // scope this config would register again: fail closed.
                return Err("security secret scope retirement pending".into());
            }
        }
    }
    if super::secret_tenants::live(&key_path).is_some() {
        return Ok(());
    }
    register_tenant(config, state, &proxy, key_path).await
}

/// Register `config`'s approval scope and its runtime secret registration,
/// with a cipher, consumer, master backend and pairing store of its own.
async fn register_tenant(
    config: &Config,
    state: &SecurityCallbackState,
    proxy: &tinybus::module::HostCallbackProxy,
    key_path: PathBuf,
) -> Result<(), String> {
    let key_dir = key_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let user_id =
        crate::security::secret_storage::encrypted_store::keyring_user_id_from_dir(&key_dir);
    let subject = tinysecurity_bus::SubjectScope {
        user: user_id.clone(),
        workspace: config.workspace_dir.to_string_lossy().into_owned(),
        agent: "config-secrets".into(),
    };
    let scope = crate::security::approval::native::prepare_secret_scope(config, subject).await?;
    let target_backend = crate::security::secret_storage::store::effective_backend_kind();
    let ids = super::secret_tenants::ids_for(&key_path, &format!("{target_backend:?}"));
    let registration_id = ids.registration_id.clone();
    log::debug!("[security] registering runtime secret scope");

    let pairing_store = super::device_pairing_store::DevicePairingStore::current(
        config,
        super::policy::pairing_backend_id(),
        registration_id.clone(),
    )
    .await
    .map_err(|error| {
        tracing::warn!(
            %error,
            workspace = %config.workspace_dir.display(),
            "[security] device pairing store could not open"
        );
        format!("device pairing persistence unavailable: {error:#}")
    })?;
    let backend = std::sync::Arc::new(super::secret_backend::SecretBackend::new(
        registration_id.clone(),
        user_id,
        ids.target_backend_id.clone(),
        ids.legacy_backend_id.clone(),
        key_path.clone(),
        target_backend,
    ));
    state
        .backends
        .add(std::sync::Arc::clone(&backend))
        .map_err(str::to_owned)?;
    if let Err(error) = state
        .consumer
        .add(registration_id.clone(), ids.consumer_id.clone())
    {
        state.backends.remove(&registration_id);
        return Err(error.to_owned());
    }
    let unroute = || {
        state.backends.remove(&registration_id);
        state.consumer.remove(&registration_id);
        // These ids may be spent in the module: the next attempt uses new ones.
        super::secret_tenants::spend_ids(&key_path);
    };

    let info: tinysecurity_bus::PolicyInfo = match proxy
        .call_confidential(tinysecurity_bus::names::methods::POLICY_INFO, ())
        .await
    {
        Ok(info) => info,
        Err(_) => {
            unroute();
            return Err("security module policy unavailable".to_owned());
        }
    };
    if let Err(error) = scope
        .ensure_registered_with_callback(config, proxy, info.generation)
        .await
    {
        unroute();
        return Err(error);
    }
    let request = super::secret_scope_request(&scope, &ids);
    let receipt: Result<tinysecurity_bus::SecretScopeReceipt2, _> = proxy
        .call_confidential(
            tinysecurity_bus::names::methods::REGISTER_SECRET_SCOPE2,
            (request,),
        )
        .await;
    let registered = matches!(
        &receipt,
        Ok(receipt) if receipt.active && receipt.registration_id == registration_id
    );
    if !registered {
        // Nothing of this registration may stay live: retire the approval
        // scope, which drops any secret registration and cipher under it.
        let _ = retire_approval_scope(proxy, &scope).await;
        unroute();
        return Err(match receipt {
            Ok(_) => "security secret scope registration refused".to_owned(),
            Err(_) => "security secret scope registration failed".to_owned(),
        });
    }

    super::callbacks::register_pairing_store(scope.scope_id().to_owned(), pairing_store.clone());
    let pairing = tinysecurity_bus::RegisterDevicePairingScopeRequest2 {
        approval_scope_id: scope.scope_id().to_owned(),
    };
    let pairing: Result<tinysecurity_bus::DevicePairingScopeReceipt2, _> = proxy
        .call_confidential(
            tinysecurity_bus::names::methods::REGISTER_DEVICE_PAIRING_SCOPE2,
            (pairing,),
        )
        .await;
    if !pairing.is_ok_and(|receipt| receipt.active && receipt.scope_id == scope.scope_id()) {
        // TinySecurity keeps one pairing scope per approval scope for the life
        // of the process, so a re-registered scope cannot pair again. The
        // module denies every pairing call for it; secrets keep working.
        log::warn!("[security] device pairing scope unavailable for this secret scope");
    }
    super::secret_tenants::insert(std::sync::Arc::new(Tenant {
        key_path,
        scope,
        ids,
        backend,
        pairing_store,
    }));
    log::info!("[security] runtime secret scope registered");
    Ok(())
}

async fn retire_approval_scope(
    proxy: &tinybus::module::HostCallbackProxy,
    scope: &crate::security::approval::native::SecretScope,
) -> Result<(), String> {
    let request = tinysecurity_bus::RetireApprovalScopeRequest2 {
        scope_id: scope.scope_id().to_owned(),
    };
    let receipt: tinysecurity_bus::ApprovalScopeReceipt2 = proxy
        .call_confidential(
            tinysecurity_bus::names::methods::RETIRE_APPROVAL_SCOPE2,
            (request,),
        )
        .await
        .map_err(|_| "security approval scope retirement failed".to_owned())?;
    // Retired either way: the next registration starts from REGISTER.
    scope.forget_registration().await;
    if receipt.scope_id != scope.scope_id() {
        return Err("TinySecurity retired another approval scope".into());
    }
    Ok(())
}

/// Retire one released tenant: its approval scope (and with it the secret
/// registration, its cipher and pending work), then every host route to it.
async fn retire_tenant(
    proxy: &tinybus::module::HostCallbackProxy,
    state: &SecurityCallbackState,
    tenant: &Tenant,
) -> Result<(), String> {
    retire_approval_scope(proxy, &tenant.scope).await?;
    let registration_id = &tenant.ids.registration_id;
    state.backends.remove(registration_id);
    state.consumer.remove(registration_id);
    super::callbacks::unregister_pairing_store(tenant.scope.scope_id());
    super::forget_unlocked_pairing_scope(tenant.scope.scope_id());
    super::secret_tenants::spend_ids(&tenant.key_path);
    log::info!("[security] runtime secret scope retired");
    Ok(())
}

#[cfg(test)]
#[path = "host_callbacks_tests.rs"]
mod tests;

//! Authenticated callback bindings retained for the admitted TinySecurity peer.

use std::path::{Path, PathBuf};

use crate::config::Config;
use tinybus::module::{HostCallbackBinding, HostCallbackEndpoint, Resolver};

struct SecurityCallbackState {
    binding: std::sync::Arc<HostCallbackBinding>,
    consumer: std::sync::Arc<super::cipher_consumer::Consumer>,
    _backend: std::sync::Arc<super::secret_backend::SecretBackend>,
    scope: crate::security::approval::native::SecretScope,
    registration_id: String,
    consumer_id: String,
    target_backend_id: String,
    legacy_backend_id: String,
    pairing_backend_id: String,
    pairing_store: super::device_pairing_store::DevicePairingStore,
    key_path: PathBuf,
}

type SecurityCallbackLoads =
    tokio::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<SecurityCallbackState>>>;

static SECURITY_CALLBACK_LOADS: std::sync::LazyLock<SecurityCallbackLoads> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(std::collections::HashMap::new()));

/// Which secret scope (its `.secret_key` path) is using each admitted module.
///
/// TinySecurity keeps the secret scope it was admitted with for the life of
/// the module: its secret registration and audit stream cannot be replaced by
/// reinitialization, and a loaded module is never unloaded. So a module is
/// handed to one scope at a time. Another scope is refused while the current
/// one is live. Once the host releases the current one ([`release_scopes`]),
/// the module may be taken over, but only by the admitting scope itself or by
/// a scope the host declared transient ([`declare_transient_scope`]): a
/// durable scope must never encrypt state it keeps under a master key that
/// belongs to a different scope.
///
/// A plain mutex, not the async one guarding [`SECURITY_CALLBACK_LOADS`], so a
/// host can release its scope from a synchronous `Drop`.
#[derive(Default)]
struct ScopeOwnership {
    /// The live owner of each admitted module; no entry once released.
    owners: std::collections::HashMap<String, PathBuf>,
    /// Scopes whose secret state does not outlive their runtime.
    transient: std::collections::HashSet<PathBuf>,
    /// Bumped by every release. A load that started before a release must
    /// not publish its scope as the owner when it finishes after it.
    epoch: u64,
}

static SECURITY_SCOPE_OWNERSHIP: std::sync::LazyLock<std::sync::Mutex<ScopeOwnership>> =
    std::sync::LazyLock::new(Default::default);

fn ownership() -> std::sync::MutexGuard<'static, ScopeOwnership> {
    SECURITY_SCOPE_OWNERSHIP
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// How a later caller came to use an admitted module's secret scope.
#[derive(Debug, PartialEq, Eq)]
enum ScopeClaim {
    /// The caller's scope is the one already using the module.
    Held,
    /// The previous owner was released; the caller's scope now uses it.
    TakenOver,
}

/// Whether the secret scope at `requested` may use a module admitted with
/// `admitted`, whose live owner is `owner` (`None`: released).
fn claim_scope(
    owner: Option<&Path>,
    admitted: &Path,
    requested: &Path,
    transient: bool,
) -> Result<ScopeClaim, String> {
    match owner {
        Some(owner) if owner == requested => Ok(ScopeClaim::Held),
        Some(_) => Err("security secret scope changed after module admission".into()),
        None if requested == admitted || transient => Ok(ScopeClaim::TakenOver),
        None => Err("security secret scope changed after module admission; \
             a durable scope cannot take over a released one"
            .into()),
    }
}

/// The current release epoch, captured before a load is scheduled.
pub(in crate::modules) fn scope_epoch() -> u64 {
    ownership().epoch
}

/// Claim module `id` for `requested`, given the scope it was admitted with.
fn claim(id: &str, admitted: &Path, requested: &Path) -> Result<ScopeClaim, String> {
    let ownership = ownership();
    claim_scope(
        ownership.owners.get(id).map(PathBuf::as_path),
        admitted,
        requested,
        ownership.transient.contains(requested),
    )
}

/// Record `key_path` as the live owner of module `id`, unless a release
/// happened since `epoch` was captured: the load that started before it
/// belongs to a released runtime. Returns whether the owner was recorded.
fn publish_owner(id: &str, key_path: PathBuf, epoch: u64) -> bool {
    let mut ownership = ownership();
    if ownership.epoch != epoch {
        log::debug!("[security] scope released during load; owner not recorded module={id}");
        return false;
    }
    ownership.owners.insert(id.to_owned(), key_path);
    true
}

fn secret_key_path(config_dir: &Path) -> PathBuf {
    config_dir.join(".secret_key")
}

/// Declare the config directory `config_dir` transient. See
/// [`crate::security::declare_transient_secret_scope`].
pub(crate) fn declare_transient_scope(config_dir: &Path) {
    ownership().transient.insert(secret_key_path(config_dir));
}

/// Release every module's live owner and forget the declared transient
/// scopes. See [`crate::security::release_secret_scope`].
pub(crate) fn release_scopes() {
    let mut ownership = ownership();
    ownership.epoch = ownership.epoch.wrapping_add(1);
    ownership.transient.clear();
    let released = ownership.owners.drain().count();
    log::debug!("[security] secret scope owners released count={released}");
}

impl SecurityCallbackState {
    fn module_config(
        &self,
        config: &Config,
        judge: Option<tinysecurity_bus::JudgeConfig>,
    ) -> Result<serde_json::Value, String> {
        let trusted_host = self.binding.unique_name().to_string();
        let config = super::module_config_for_secret_scope(
            config,
            Some(trusted_host),
            judge,
            &self.scope,
            self.registration_id.clone(),
            self.consumer_id.clone(),
            self.target_backend_id.clone(),
            self.legacy_backend_id.clone(),
            self.pairing_backend_id.clone(),
        )
        .map_err(str::to_owned)?;
        serde_json::to_value(config).map_err(|_| "invalid security module configuration".into())
    }
}

/// The secret key path TinySecurity serves, if it is admitted: its live
/// owner, or once released the admitting scope, the only durable scope that
/// may take it over ([`claim_scope`]).
pub(in crate::modules) async fn admitted_key_path() -> Option<PathBuf> {
    let admitted = SECURITY_CALLBACK_LOADS
        .lock()
        .await
        .get(super::MODULE_ID)
        .map(|state| state.key_path.clone())?;
    Some(
        ownership()
            .owners
            .get(super::MODULE_ID)
            .cloned()
            .unwrap_or(admitted),
    )
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

pub(in crate::modules) async fn secret_backend(
) -> Option<std::sync::Arc<super::secret_backend::SecretBackend>> {
    SECURITY_CALLBACK_LOADS
        .lock()
        .await
        .get(super::MODULE_ID)
        .map(|state| std::sync::Arc::clone(&state._backend))
}

pub(in crate::modules) async fn secret_scope(
) -> Option<crate::security::approval::native::SecretScope> {
    SECURITY_CALLBACK_LOADS
        .lock()
        .await
        .get(super::MODULE_ID)
        .map(|state| state.scope.clone())
}

pub(in crate::modules) async fn secret_registration_id() -> Option<String> {
    SECURITY_CALLBACK_LOADS
        .lock()
        .await
        .get(super::MODULE_ID)
        .map(|state| state.registration_id.clone())
}

pub(in crate::modules) async fn identity() -> Option<String> {
    SECURITY_CALLBACK_LOADS
        .lock()
        .await
        .get(super::MODULE_ID)
        .map(|state| state.binding.unique_name().to_string())
}

pub(in crate::modules) async fn module_config(
    config: &Config,
) -> Result<serde_json::Value, String> {
    let judge = super::judge::resolve(config);
    let loads = SECURITY_CALLBACK_LOADS.lock().await;
    loads
        .get(super::MODULE_ID)
        .ok_or_else(|| "security callback binding unavailable".to_owned())?
        .module_config(config, judge)
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

fn opaque_id(label: &str, path: &Path, suffix: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(label.as_bytes());
    hasher.update((path.as_os_str().len() as u64).to_be_bytes());
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update(suffix.as_bytes());
    format!("{:x}", hasher.finalize())[..32].to_owned()
}

/// Admit module `id` for `config`'s secret scope, or confirm the admitted one
/// may serve it. `epoch` is [`scope_epoch`] captured when the caller asked,
/// before the load was scheduled.
pub(in crate::modules) async fn ensure_loaded(
    resolver: &Resolver,
    config: &Config,
    id: &str,
    epoch: u64,
) -> Result<(), String> {
    let mut loads = SECURITY_CALLBACK_LOADS.lock().await;
    if let Some(state) = loads.get(id) {
        let key_path = secret_key_path(
            config
                .config_path
                .parent()
                .unwrap_or_else(|| Path::new(".")),
        );
        let claim = claim(id, &state.key_path, &key_path)?;
        // Confirm the admitted module still retains this binding. Re-presenting
        // it would be refused (a binding is spent once a module takes it), and
        // the single-binding path accepts only a local-file source, so it
        // failed every later call on a host served from the release.
        super::super::ops::callback_load::confirm(resolver, id, &state.binding).await?;
        register_scope(config, state).await?;
        // The loads lock is held, so no other caller claims it meanwhile.
        if claim == ScopeClaim::TakenOver && publish_owner(id, key_path, epoch) {
            log::info!(
                "[security] secret scope taken over after its owner was released module={id}"
            );
        }
        return Ok(());
    }

    let key_dir = config
        .config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let key_path = secret_key_path(&key_dir);
    let user_id =
        crate::security::secret_storage::encrypted_store::keyring_user_id_from_dir(&key_dir);
    let subject = tinysecurity_bus::SubjectScope {
        user: user_id.clone(),
        workspace: config.workspace_dir.to_string_lossy().into_owned(),
        agent: "config-secrets".into(),
    };
    let scope = crate::security::approval::native::prepare_secret_scope(config, subject).await?;

    let target_backend = crate::security::secret_storage::store::effective_backend_kind();
    let target_backend_id = opaque_id("secret-backend", &key_path, &format!("{target_backend:?}"));
    let legacy_backend_id = opaque_id("secret-legacy", &key_path, "file");
    let registration_id = opaque_id("secret-registration", &key_path, "config-user-master");
    let consumer_id = opaque_id("secret-cipher-consumer", &key_path, "config-user-master");
    let pairing_backend_id = opaque_id("device-pairing-state", &key_path, "encrypted-snapshot");

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
    let factory_scope = scope.clone();
    let factory_candidates = std::sync::Arc::clone(&candidates);
    let factory_key_path = key_path.clone();
    let factory_user = user_id.clone();
    let factory_registration = registration_id.clone();
    let factory_consumer_id = consumer_id.clone();
    let factory_target_backend_id = target_backend_id.clone();
    let factory_legacy_backend_id = legacy_backend_id.clone();
    let factory_pairing_backend_id = pairing_backend_id.clone();

    let binding = resolver
        .ensure_loaded_with_host_callback_factory(id, move |_attempt| {
            let host = host;
            let config = factory_config.clone();
            let scope = factory_scope.clone();
            let candidates = std::sync::Arc::clone(&factory_candidates);
            let key_path = factory_key_path.clone();
            let user_id = factory_user.clone();
            let registration_id = factory_registration.clone();
            let consumer_id = factory_consumer_id.clone();
            let target_backend_id = factory_target_backend_id.clone();
            let legacy_backend_id = factory_legacy_backend_id.clone();
            let pairing_backend_id = factory_pairing_backend_id.clone();
            async move {
                let binding = std::sync::Arc::new(
                    host.prepare_host_callbacks(callback_endpoints()?)
                        .await
                        .map_err(|_| "security callback binding unavailable".to_owned())?,
                );
                let consumer = std::sync::Arc::new(super::cipher_consumer::Consumer::new(
                    registration_id.clone(),
                    consumer_id.clone(),
                ));
                let backend = std::sync::Arc::new(super::secret_backend::SecretBackend::new(
                    registration_id.clone(),
                    user_id,
                    target_backend_id.clone(),
                    legacy_backend_id.clone(),
                    key_path.clone(),
                    target_backend,
                ));
                super::serve_secret_callbacks(
                    &binding,
                    std::sync::Arc::clone(&consumer),
                    std::sync::Arc::clone(&backend),
                )
                .await
                .map_err(|_| "security callback registration failed".to_owned())?;
                let pairing_store = super::device_pairing_store::DevicePairingStore::current(
                    &config,
                    pairing_backend_id.clone(),
                    registration_id.clone(),
                )
                .await
                .map_err(|_| "device pairing persistence unavailable".to_owned())?;
                let state = std::sync::Arc::new(SecurityCallbackState {
                    binding: std::sync::Arc::clone(&binding),
                    consumer,
                    _backend: backend,
                    scope,
                    registration_id,
                    consumer_id,
                    target_backend_id,
                    legacy_backend_id,
                    pairing_backend_id,
                    pairing_store,
                    key_path,
                });
                super::callbacks::register_pairing_store(
                    state.scope.scope_id().to_owned(),
                    state.pairing_store.clone(),
                );
                let judge = super::judge::resolve(&config);
                let module_config = state.module_config(&config, judge)?;
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
    publish_owner(id, state.key_path.clone(), epoch);

    register_scope(config, &state).await?;
    Ok(())
}

async fn register_scope(config: &Config, state: &SecurityCallbackState) -> Result<(), String> {
    let record = super::super::registry::find(super::MODULE_ID)
        .ok_or_else(|| "security module registry record missing".to_owned())?;
    let proxy = state
        .binding
        .proxy(
            record.bus_name,
            record.object_path,
            tinysecurity_bus::names::INTERFACE,
        )
        .map_err(|_| "security module proxy unavailable".to_owned())?;
    let info: tinysecurity_bus::PolicyInfo = proxy
        .call_confidential(tinysecurity_bus::names::methods::POLICY_INFO, ())
        .await
        .map_err(|_| "security module policy unavailable".to_owned())?;
    state
        .scope
        .ensure_registered_with_callback(config, &proxy, info.generation)
        .await?;
    let request = tinysecurity_bus::RegisterDevicePairingScopeRequest2 {
        approval_scope_id: state.scope.scope_id().to_owned(),
    };
    let receipt: tinysecurity_bus::DevicePairingScopeReceipt2 = proxy
        .call_confidential(
            tinysecurity_bus::names::methods::REGISTER_DEVICE_PAIRING_SCOPE2,
            (request,),
        )
        .await
        .map_err(|_| "device pairing scope registration failed".to_owned())?;
    if !receipt.active || receipt.scope_id != state.scope.scope_id() {
        return Err("device pairing scope registration refused".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "host_callbacks_tests.rs"]
mod tests;

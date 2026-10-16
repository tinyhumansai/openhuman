//! Binding the configured memory engine.
//!
//! [`resolve`] turns the `[memory]` config into either a live engine
//! ([`Binding::On`]) or the reason memory is off ([`Binding::Off`]):
//!
//! | Engine | Endpoint | Credential | Off when |
//! | --- | --- | --- | --- |
//! | `tinyhumans` | registered by the host layer that owns it ([`super::engine_registry`]) | the host's backend credential, resolved per request | not registered, or signed out |
//! | `cortexdb` | `[memory.engines.cortexdb] endpoint`, else CortexDB's managed API | the API key stored as [`MEMORY_CORTEXDB_KEY_NAME`] | no key stored |
//!
//! Built engines are cached per config fingerprint (engine id, endpoint,
//! credential identity, layout), so a config change, a sign-in or a new key
//! rebuilds the engine on the next call without any event plumbing, and
//! repeated calls reuse one HTTP client.
//!
//! **Layout.** Under `[memory] layout = "v3"` the engine keeps everything
//! below the signed-in person's own scope root (`org:<id>`, see
//! [`super::scope::user_root`]): on CortexDB's own API as its root,
//! registered as owned by their actor `user:<id>`; on the TinyHumans wire as
//! the tenant root the backend pins, so no root segment is sent. While
//! `[memory] legacy_user_segment_read` is on, the earlier `user:<id>` root is
//! read and forgotten as well. Otherwise the legacy shared tree. [`bind_with_root`] binds either layout explicitly,
//! for the layout migration, which holds both at once.
//!
//! An embedding host can bring its own engine instead ([`install_host_engine`]):
//! it then wins over the configured one for every config in the process — one
//! OpenHuman runtime per process — so a host with its own store, or a test
//! binary with an in-memory engine, runs the whole lifecycle without a
//! TinyHumans credential or a CortexDB key. It is wrapped in the same
//! scrubber as a configured engine.
//!
//! The core never holds a TinyHumans credential of its own: a registered
//! engine's bearer comes from the host's credential seam on every request, so
//! a refreshed session is used at once and signing out turns memory off.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use sha2::{Digest, Sha256};
use tinymemory_api::MemoryEngine;
pub use tinymemory_integrations::{BearerSource, EngineCredential, EngineSettings};

use crate::config::schema::MEMORY_CORTEXDB_KEY_NAME;
use crate::config::Config;
use crate::security::credentials::session_support::resolve_backend_credential;
use crate::security::credentials::{AuthService, DEFAULT_AUTH_PROFILE_NAME};

use super::error::{MemoryError, MemoryResult};

/// Engine id of CortexDB behind the TinyHumans backend.
pub const TINYHUMANS_ENGINE: &str = tinymemory_integrations::cortex::TINYHUMANS_ENGINE_ID;

/// Engine id of CortexDB reached directly.
pub const CORTEXDB_ENGINE: &str = tinymemory_integrations::cortex::CORTEXDB_ENGINE_ID;

/// Pseudo-engine id for memory turned off on purpose: nothing is bound, so
/// nothing is stored or recalled until another engine is selected.
pub const DISABLED_ENGINE: &str = "none";

/// A bound engine. Its writes are scrubbed ([`super::guard`]).
#[derive(Clone)]
pub struct BoundEngine {
    /// The engine.
    pub engine: Arc<dyn MemoryEngine>,
    /// Its id.
    pub id: String,
    /// The endpoint it talks to.
    pub endpoint: String,
}

impl std::fmt::Debug for BoundEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoundEngine")
            .field("id", &self.id)
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

/// Whether memory is on, and with what.
#[derive(Debug, Clone)]
pub enum Binding {
    /// An engine is bound.
    On(BoundEngine),
    /// Memory is off.
    Off {
        /// The configured engine id, when it names a known engine.
        engine: Option<String>,
        /// The configured endpoint, when known.
        endpoint: Option<String>,
        /// Why memory is off (no credential detail).
        reason: String,
    },
}

impl Binding {
    /// The bound engine, or [`MemoryError::Off`].
    pub fn engine(self) -> MemoryResult<BoundEngine> {
        match self {
            Self::On(bound) => Ok(bound),
            Self::Off { reason, .. } => Err(MemoryError::Off(reason)),
        }
    }

    /// Whether an engine is bound.
    #[must_use]
    pub fn is_on(&self) -> bool {
        matches!(self, Self::On(_))
    }
}

/// Built engines, keyed by fingerprint.
static CACHE: LazyLock<RwLock<HashMap<String, BoundEngine>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Test engines, keyed by workspace directory, so concurrent tests that each
/// own a temp workspace never see one another's engine.
#[cfg(test)]
static TEST_ENGINES: LazyLock<RwLock<HashMap<std::path::PathBuf, Arc<dyn MemoryEngine>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Binds `engine` for every config whose workspace is `workspace` (tests only).
#[cfg(test)]
pub(crate) fn install_test_engine(workspace: &std::path::Path, engine: Arc<dyn MemoryEngine>) {
    TEST_ENGINES
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(workspace.to_path_buf(), engine);
}

/// Unbinds the test engine of `workspace`, so memory there is off again
/// (tests only).
#[cfg(test)]
pub(crate) fn remove_test_engine(workspace: &std::path::Path) {
    TEST_ENGINES
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(workspace);
}

/// Test engines for one layout of a workspace (`None` legacy, `Some(root)`
/// v3), read by [`bind_with_root`] ahead of everything else, so a test can
/// hold both layouts at once as the layout migration does.
#[cfg(test)]
type RootEngines = HashMap<(std::path::PathBuf, Option<String>), Arc<dyn MemoryEngine>>;

#[cfg(test)]
static TEST_ROOT_ENGINES: LazyLock<RwLock<RootEngines>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Binds `engine` for [`bind_with_root`] with `root` on configs whose
/// workspace is `workspace` (tests only).
#[cfg(test)]
pub(crate) fn install_test_engine_for_root(
    workspace: &std::path::Path,
    root: Option<&str>,
    engine: Arc<dyn MemoryEngine>,
) {
    TEST_ROOT_ENGINES
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert((workspace.to_path_buf(), root.map(str::to_string)), engine);
}

/// The engine an embedding host installed, if any ([`install_host_engine`]).
static HOST_ENGINE: LazyLock<RwLock<Option<BoundEngine>>> = LazyLock::new(|| RwLock::new(None));

/// Endpoint reported for a host-installed engine.
pub const HOST_ENGINE_ENDPOINT: &str = "host://engine";

/// Binds `engine` for every config in this process, ahead of the configured
/// one. For an embedding host that owns its memory store; the engine's writes
/// are scrubbed like any other's ([`super::guard`]). A later call replaces the
/// earlier engine.
pub fn install_host_engine(engine: Arc<dyn MemoryEngine>) {
    let id = engine.descriptor().id.to_string();
    tracing::info!(engine = %id, "[memory:engine] host engine installed");
    *HOST_ENGINE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(BoundEngine {
        id,
        endpoint: HOST_ENGINE_ENDPOINT.to_string(),
        engine: super::guard::ScrubbingEngine::wrap(engine),
    });
}

/// Removes a host-installed engine; the configured engine applies again.
/// Returns whether one was installed.
pub fn clear_host_engine() -> bool {
    HOST_ENGINE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
        .is_some()
}

fn host_engine() -> Option<BoundEngine> {
    HOST_ENGINE
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// Resolves the configured engine.
#[must_use]
pub fn resolve(config: &Config) -> Binding {
    if !config.components.memory {
        tracing::debug!("[memory:engine] components.memory is off; engine not bound");
        return off(None, None, COMPONENT_OFF_REASON);
    }
    // The hosted profile's engine is bound for the calling task's own tenant.
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    if let Some(capabilities) = crate::profiles::capabilities::ProfileCapabilities::in_ctx(&ctx) {
        if let Some(engine) = &capabilities.memory_engine {
            if let Err(reason) = crate::profiles::host::ensure_hosted(&ctx) {
                return off(None, None, &reason);
            }
            let profile = ctx.tenant().ok().and_then(|tenant| tenant.profile);
            let valid = profile.as_ref().is_some_and(|id| {
                config.memory.agent_id.as_ref() == Some(id)
                    && config.memory.root.as_deref() == Some(format!("user:{id}").as_str())
            });
            if !valid {
                return off(
                    None,
                    None,
                    "profile memory binding requires its forced namespace",
                );
            }
            return Binding::On(BoundEngine {
                id: engine.descriptor().id.to_string(),
                endpoint: HOST_ENGINE_ENDPOINT.to_string(),
                engine: super::guard::ScrubbingEngine::wrap(Arc::clone(engine)),
            });
        }
    }
    #[cfg(test)]
    {
        let installed = TEST_ENGINES
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&config.workspace_dir)
            .cloned();
        if let Some(engine) = installed {
            return Binding::On(BoundEngine {
                id: engine.descriptor().id.to_string(),
                endpoint: "test://engine".to_string(),
                engine: super::guard::ScrubbingEngine::wrap(engine),
            });
        }
    }
    if let Some(bound) = host_engine() {
        // A host engine is one store for the whole process: in SaaS it would
        // put every user in one engine, so it is ignored there.
        if admits_host_engine_with(crate::core::runtime::is_saas()) {
            return Binding::On(bound);
        }
        tracing::warn!("[memory:engine] ignoring the host engine in SaaS mode");
    }
    let root = if super::scope::layout_is_v3(config) {
        match super::scope::user_root(config) {
            Some(root) => Some(root),
            None => return off(None, None, "sign in to use memory in its own layout"),
        }
    } else {
        None
    };
    resolve_configured(config, root.as_deref())
}

/// The engine the layout migration needs, for an explicit layout whatever
/// `[memory] layout` says: `None` the legacy tree, `Some("org:<id>")` the
/// person's v3 subtree, with the actor `user:<id>` as its owner. Same endpoint,
/// credential, headers and scrubbing as [`resolve`]; the two layouts are
/// cached apart, so both can be held at once.
///
/// # Errors
///
/// Memory is off for `config`, or an embedding host's (or a test's)
/// installed engine is bound, which has one layout of its own and cannot be
/// bound below a scope root.
pub fn bind_with_root(config: &Config, root: Option<&str>) -> MemoryResult<BoundEngine> {
    if !config.components.memory {
        return Err(MemoryError::Off(COMPONENT_OFF_REASON.to_string()));
    }
    #[cfg(test)]
    {
        let installed = TEST_ROOT_ENGINES
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&(config.workspace_dir.clone(), root.map(str::to_string)))
            .cloned();
        if let Some(engine) = installed {
            return Ok(BoundEngine {
                id: engine.descriptor().id.to_string(),
                endpoint: "test://engine".to_string(),
                engine: super::guard::ScrubbingEngine::wrap(engine),
            });
        }
        let one_engine = TEST_ENGINES
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(&config.workspace_dir);
        if one_engine {
            return match root {
                None => resolve(config).engine(),
                Some(_) => Err(MemoryError::Engine(
                    "the installed test engine cannot be bound below a scope root".to_string(),
                )),
            };
        }
    }
    // As in `resolve`: a SaaS process never hands out the process-wide host
    // engine, which every user would share.
    if let Some(bound) = host_binding_with(crate::core::runtime::is_saas(), host_engine(), root)? {
        return Ok(bound);
    }
    resolve_configured(config, root).engine()
}

/// Whether the process-wide host engine may be used: never in SaaS, where one
/// engine would put every user in the same store.
pub(crate) fn admits_host_engine_with(saas: bool) -> bool {
    !saas
}

/// The host engine `bind_with_root` returns for `root`, if any. `host` is
/// ignored in SaaS; outside it the host engine has one layout of its own, so
/// it cannot be bound below a scope root.
pub(crate) fn host_binding_with<T>(
    saas: bool,
    host: Option<T>,
    root: Option<&str>,
) -> MemoryResult<Option<T>> {
    let host = if admits_host_engine_with(saas) {
        host
    } else {
        None
    };
    if host.is_some() && root.is_some() {
        return Err(MemoryError::Engine(
            "the host's memory engine cannot be bound below a scope root".to_string(),
        ));
    }
    Ok(if root.is_none() { host } else { None })
}

/// The configured engine with its scope root (`None` legacy).
fn resolve_configured(config: &Config, root: Option<&str>) -> Binding {
    let engine_id = config.memory.engine.trim().to_string();
    // A registered engine (`engine_registry`) wins over the built-in match, and
    // is resolved per config, so SaaS admits it for every tenant.
    if let Some(entry) = super::engine_registry::lookup(&engine_id) {
        tracing::debug!(engine = %engine_id, "[memory:engine] binding a registered engine");
        return (entry.build)(&super::engine_registry::EngineRequest { config, root });
    }
    match engine_id.as_str() {
        // The hosted engine is registered by the host layer that owns it
        // (`engine_registry`); without one it is simply unavailable.
        TINYHUMANS_ENGINE => off(
            Some(TINYHUMANS_ENGINE),
            None,
            "no TinyHumans backend is available",
        ),
        CORTEXDB_ENGINE => resolve_cortexdb(config, root),
        DISABLED_ENGINE => off(None, None, "memory is disabled"),
        "" => {
            let reason = if config.memory.legacy_backend_unsupported
                || config.memory.legacy_backend.is_some()
            {
                "legacy memory backend is unsupported; select an explicit v2 memory engine"
                    .to_string()
            } else {
                "no memory engine selected".to_string()
            };
            off(None, None, &reason)
        }
        other => {
            tracing::warn!(engine = %other, "[memory:engine] unknown engine id in config");
            off(
                None,
                None,
                "the configured memory engine is not available in this build",
            )
        }
    }
}

/// Why memory answers off while `[components] memory = false`.
const COMPONENT_OFF_REASON: &str = "memory component is disabled ([components] memory = false)";

/// Whether memory is switched off by config: `[components] memory = false` or
/// `[memory] engine = "none"`. No engine is bound in either case.
#[must_use]
pub fn switched_off(config: &Config) -> bool {
    !config.components.memory || config.memory.engine == DISABLED_ENGINE
}

/// Whether memory is on for `config`.
#[must_use]
pub fn is_on(config: &Config) -> bool {
    resolve(config).is_on()
}

fn off(engine: Option<&str>, endpoint: Option<String>, reason: &str) -> Binding {
    Binding::Off {
        engine: engine.map(str::to_string),
        endpoint,
        reason: reason.to_string(),
    }
}

/// The engine settings' layout for the person's root `root` (`org:<id>`,
/// `None` the legacy tree), and the fingerprint suffix that keeps each
/// layout's engine apart in the cache. On the TinyHumans wire (`tenant`)
/// the root is the tenant's own, pinned by the backend, so none is sent;
/// on CortexDB's own API it is the root, owned by the actor `user:<id>`.
/// With `legacy_read`, the earlier `user:<id>` root is still read and
/// forgotten (never written).
pub fn rooted(
    settings: EngineSettings,
    root: Option<&str>,
    tenant: bool,
    legacy_read: bool,
) -> (EngineSettings, String) {
    let Some(root) = root else {
        return (settings, "|root=legacy".to_string());
    };
    let actor = super::scope::actor_of_root(root);
    let retired = legacy_read.then(|| actor.clone());
    let fingerprint = format!(
        "|root={root}|tenant={tenant}|retired={}",
        retired.as_deref().unwrap_or("-")
    );
    let settings = if tenant {
        EngineSettings {
            tenant_root: true,
            retired_scope_root: retired,
            ..settings
        }
    } else {
        EngineSettings {
            scope_root: Some(root.to_string()),
            scope_owner: Some(actor),
            retired_scope_root: retired,
            ..settings
        }
    };
    tracing::debug!(
        tenant,
        legacy_read,
        "[memory:engine] binding layout v3 below the person's org root"
    );
    (settings, fingerprint)
}

fn resolve_cortexdb(config: &Config, root: Option<&str>) -> Binding {
    let configured = config.memory.endpoint_for(CORTEXDB_ENGINE);
    let endpoint = configured
        .clone()
        .unwrap_or_else(|| tinymemory_integrations::cortex::CORTEX_API_ENDPOINT.to_string());
    let key = match read_cortexdb_key(config) {
        Ok(Some(key)) => key,
        Ok(None) => {
            return off(
                Some(CORTEXDB_ENGINE),
                Some(endpoint),
                "add a CortexDB API key to use CortexDB memory",
            )
        }
        Err(error) => {
            tracing::warn!(error = %error, "[memory:engine] reading the CortexDB key failed");
            return off(
                Some(CORTEXDB_ENGINE),
                Some(endpoint),
                "the CortexDB API key could not be read",
            );
        }
    };
    let observed_actor = config.memory.observed_actor;
    let fingerprint = format!(
        "{CORTEXDB_ENGINE}|{endpoint}|{}|actor={observed_actor}",
        key_digest(&key)
    );
    // A third-party endpoint: no TinyHumans attribution headers.
    let (settings, layout) = rooted(
        EngineSettings {
            endpoint: Some(endpoint),
            observed_actor,
            ..EngineSettings::default()
        },
        root,
        false,
        config.memory.legacy_user_segment_read,
    );
    build_cached(
        CORTEXDB_ENGINE,
        settings,
        fingerprint + &layout,
        EngineCredential::Static(key),
    )
}

pub fn build_cached(
    id: &str,
    settings: EngineSettings,
    fingerprint: String,
    credential: EngineCredential,
) -> Binding {
    let endpoint = settings.endpoint.clone().unwrap_or_default();
    if let Some(bound) = CACHE
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&fingerprint)
        .cloned()
    {
        return Binding::On(bound);
    }
    match tinymemory_integrations::build_engine(id, &settings, credential) {
        Ok(engine) => {
            tracing::info!(engine = %id, "[memory:engine] engine bound");
            let bound = BoundEngine {
                engine: super::guard::ScrubbingEngine::wrap(engine),
                id: id.to_string(),
                endpoint: endpoint.to_string(),
            };
            CACHE
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(fingerprint, bound.clone());
            Binding::On(bound)
        }
        Err(error) => {
            tracing::warn!(engine = %id, error = %error, "[memory:engine] engine build refused");
            off(Some(id), Some(endpoint.to_string()), &error.to_string())
        }
    }
}

/// A short, non-reversible identity of a key for the cache fingerprint.
fn key_digest(key: &str) -> String {
    let digest = Sha256::digest(key.as_bytes());
    digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Held for writing by a test that compares cached engines by identity, and
/// for reading by [`invalidate`], so another test's invalidation (a layout
/// switch, a key change) cannot clear the cache between its two binds.
#[cfg(test)]
pub(crate) static CACHE_STABLE: RwLock<()> = RwLock::new(());

/// Drops every cached engine, so the next [`resolve`] rebuilds.
pub fn invalidate() {
    #[cfg(test)]
    let _stable = CACHE_STABLE
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    CACHE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    super::lifecycle::prefetch::invalidate_all();
}

/// The stored CortexDB API key, if any.
pub fn read_cortexdb_key(config: &Config) -> anyhow::Result<Option<String>> {
    AuthService::from_config(config).get_provider_bearer_token(MEMORY_CORTEXDB_KEY_NAME, None)
}

/// Stores the CortexDB API key in the credential store (never in config).
pub fn store_cortexdb_key(config: &Config, key: &str) -> MemoryResult<()> {
    let key = key.trim();
    if key.is_empty() {
        return Err(MemoryError::invalid("the API key is blank"));
    }
    AuthService::from_config(config)
        .store_provider_token(
            MEMORY_CORTEXDB_KEY_NAME,
            DEFAULT_AUTH_PROFILE_NAME,
            key,
            HashMap::new(),
            true,
        )
        .map_err(|error| MemoryError::Engine(format!("storing the API key failed: {error}")))?;
    tracing::debug!("[memory:engine] cortexdb key stored");
    Ok(())
}

/// Removes the stored CortexDB API key.
pub fn clear_cortexdb_key(config: &Config) -> MemoryResult<bool> {
    AuthService::from_config(config)
        .remove_profile(MEMORY_CORTEXDB_KEY_NAME, DEFAULT_AUTH_PROFILE_NAME)
        .map_err(|error| MemoryError::Engine(format!("removing the API key failed: {error}")))
}

/// Whether the engine `id` has a credential available.
#[must_use]
pub fn has_key(config: &Config, id: &str) -> bool {
    match id {
        TINYHUMANS_ENGINE => resolve_backend_credential(config).is_ok(),
        CORTEXDB_ENGINE => matches!(read_cortexdb_key(config), Ok(Some(_))),
        _ => false,
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;

//! Calling the `tinyruntime` module: resolving a language runtime and running
//! code on it, over the bus.
//!
//! Everything the core used to own about managed toolchains — probing the host
//! for a compatible interpreter, downloading a distribution, verifying its
//! digest, unpacking it, keeping it across restarts, and pooling warm worker
//! processes in front of it — lives in that module now. What is left here is the
//! host half of four calls.
//!
//! # Three modules, one call
//!
//! `tinyruntime` is a router and knows no languages on its own; the language
//! knowledge is in `tinyruntime-nodejs` and `tinyruntime-python`. So a call for
//! JavaScript needs two modules loaded, not one, and [`ensure_language`] loads
//! both. Load order does not matter — the router contacts providers per call
//! rather than at setup — but a router without its provider reports the language
//! unavailable, which is a confusing way to discover a missing module.
//!
//! # Configuration travels with the call
//!
//! The module holds none of its own. Every request carries the version pin, the
//! cache directory, and the pool tuning it should be served under, which is why
//! [`settings_for`] reads this host's config on each call rather than at load.
//! A user who changes `node.version` sees it take effect on the next run instead
//! of on the next restart.
//!
//! # Deadlines belong to the caller
//!
//! Nothing here imposes one. An install is a multi-hundred-megabyte download and
//! a caller that wants to bound it knows what it is willing to wait; a deadline
//! chosen here would make the effective limit the smaller of two numbers nobody
//! picked together. `Execute` carries its own per-job deadline in the request,
//! which the worker honours and reports back.

use tinyruntime_bus::{
    names, ExecRequest, ExecResponse, Language, LanguagesResponse, PoolSettings, PoolStatsResponse,
    ResolveRequest, ResolveResponse, ResolvedRuntime, RuntimeSettings,
};

use super::{host, ops, registry};
use crate::openhuman::config::Config;

/// Registry id of the router.
pub const MODULE_ID: &str = "tinyruntime";

/// Registry id of the Node.js provider.
pub const NODEJS_PROVIDER_ID: &str = "tinyruntime-nodejs";

/// Registry id of the Python provider.
pub const PYTHON_PROVIDER_ID: &str = "tinyruntime-python";

/// Why a runtime call did not produce what was asked for.
///
/// Three variants rather than one string because callers act on them
/// differently: a missing module disables a feature, a bad request is worth
/// reporting to whoever made it, and a failure mid-flight is worth retrying or
/// surfacing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeCallError {
    /// The module is not loaded and cannot be: no artifact for this host,
    /// downloads off, disabled in config, or a load that already failed in this
    /// process.
    Unavailable(String),
    /// The request was rejected — an unknown language, a version that is not one,
    /// a language the host has disabled.
    InvalidRequest(String),
    /// Resolution, installation, or execution failed.
    Failed(String),
}

impl std::fmt::Display for RuntimeCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) | Self::InvalidRequest(message) | Self::Failed(message) => {
                f.write_str(message)
            }
        }
    }
}

impl std::error::Error for RuntimeCallError {}

/// The registry id of the provider module serving `language`, if this build
/// ships one.
///
/// A language with no provider record is not an error here: the router accepts
/// any language its own configuration routes, and a host may have put a
/// third-party provider on the module search path.
#[must_use]
pub fn provider_id(language: &Language) -> Option<&'static str> {
    match language.as_str() {
        tinyruntime_bus::NODEJS => Some(NODEJS_PROVIDER_ID),
        tinyruntime_bus::PYTHON => Some(PYTHON_PROVIDER_ID),
        _ => None,
    }
}

/// Load the router and, when this build ships one, `language`'s provider.
///
/// # Errors
///
/// [`RuntimeCallError::Unavailable`] naming which of the two could not be
/// loaded.
pub async fn ensure_language(config: &Config, language: &Language) -> Result<(), RuntimeCallError> {
    ops::ensure_loaded(config, MODULE_ID)
        .await
        .map_err(RuntimeCallError::Unavailable)?;
    if let Some(provider) = provider_id(language) {
        ops::ensure_loaded(config, provider)
            .await
            .map_err(RuntimeCallError::Unavailable)?;
    }
    Ok(())
}

/// Resolve a runtime for `language`, installing one when `install` allows it.
///
/// Returns `None` when `install` is `false` and nothing is provisioned yet —
/// which is the whole point of a non-installing probe: it answers "is this
/// ready?" without committing the caller to a download.
///
/// # Errors
///
/// [`RuntimeCallError`] describing whether the module, the request, or the
/// resolution was at fault.
pub async fn resolve(
    config: &Config,
    language: &Language,
    install: bool,
) -> Result<Option<ResolvedRuntime>, RuntimeCallError> {
    ensure_language(config, language).await?;
    let settings = settings_for(config, language);
    let request = if install {
        ResolveRequest::new(language.clone(), settings)
    } else {
        ResolveRequest::probe(language.clone(), settings)
    };

    let response: ResolveResponse = call(names::methods::RESOLVE, (request,)).await?;
    Ok(response.runtime)
}

/// Run `code` on `language`, resolving and provisioning it first.
///
/// # Errors
///
/// [`RuntimeCallError`]. Note that a job which *ran* and threw is not an error:
/// it comes back as an [`ExecResponse`] with a non-zero exit code, because that
/// is output the caller wants rather than a failure of this call.
pub async fn execute(
    config: &Config,
    language: &Language,
    code: impl Into<String>,
    cwd: Option<String>,
    timeout: Option<std::time::Duration>,
) -> Result<ExecResponse, RuntimeCallError> {
    ensure_language(config, language).await?;

    let mut request = ExecRequest::new(language.clone(), settings_for(config, language), code);
    request.pool = pool_settings_for(config, language);
    request.cwd = cwd;
    request.timeout_ms =
        timeout.map(|budget| u64::try_from(budget.as_millis()).unwrap_or(u64::MAX));

    call(names::methods::EXECUTE, (request,)).await
}

/// Every language the router can route to, and whether it currently can.
///
/// Loads the router but not the providers: the point of this call is to find out
/// which providers are there, and loading them first would make the answer
/// always yes.
///
/// # Errors
///
/// [`RuntimeCallError::Unavailable`] when the router itself cannot be loaded.
pub async fn languages(config: &Config) -> Result<LanguagesResponse, RuntimeCallError> {
    ops::ensure_loaded(config, MODULE_ID)
        .await
        .map_err(RuntimeCallError::Unavailable)?;
    call(names::methods::LANGUAGES, ()).await
}

/// Every live worker pool's counters.
///
/// # Errors
///
/// [`RuntimeCallError::Unavailable`] when the router cannot be loaded.
pub async fn pool_stats(config: &Config) -> Result<PoolStatsResponse, RuntimeCallError> {
    ops::ensure_loaded(config, MODULE_ID)
        .await
        .map_err(RuntimeCallError::Unavailable)?;
    call(names::methods::POOL_STATS, ()).await
}

/// The settings this host wants `language` served under.
///
/// Read per call rather than cached, because the module holds no configuration
/// of its own: a user who edits a version pin sees it on the next run.
#[must_use]
pub fn settings_for(config: &Config, language: &Language) -> RuntimeSettings {
    match language.as_str() {
        tinyruntime_bus::PYTHON => {
            let python = &config.runtime_python;
            let mut settings = RuntimeSettings::new(python.minimum_version.clone());
            settings.enabled = python.enabled;
            settings.prefer_system = python.prefer_system;
            settings.maximum_version = python.maximum_version.clone();
            settings.cache_dir = python.cache_dir.clone();
            settings.release_tag = python.managed_release_tag.clone();
            settings.preferred_command = python.preferred_command.clone();
            settings
        }
        // Node.js is the default rather than a match arm of its own: a language
        // this build ships no configuration block for still gets a usable
        // request, and the router refuses it by name if nothing routes it.
        _ => {
            let node = &config.node;
            let mut settings = RuntimeSettings::new(node.version.clone());
            settings.enabled = node.enabled;
            settings.prefer_system = node.prefer_system;
            settings.cache_dir = node.cache_dir.clone();
            settings
        }
    }
}

/// The pool tuning this host wants for `language`.
///
/// Python defaults off where Node defaults on, and that asymmetry is real rather
/// than an oversight: a pooled Node job runs in its own worker thread with a
/// fresh module graph, while a pooled Python job shares the interpreter with
/// every other job on that worker. Opting into the second is a decision.
#[must_use]
pub fn pool_settings_for(config: &Config, language: &Language) -> PoolSettings {
    let pool = &config.runtime_pool;
    let (lang_config, default_enabled) = match language.as_str() {
        tinyruntime_bus::PYTHON => (&pool.python, false),
        _ => (&pool.node, true),
    };

    let mut settings = PoolSettings::default();
    settings.enabled = pool.enabled && lang_config.is_enabled(default_enabled);
    settings.max_workers = lang_config.effective_max_workers();
    settings.idle_ttl_secs = lang_config.idle_ttl_secs;
    settings.recycle_after_jobs = lang_config.recycle_after_jobs;
    settings.max_queue_depth = lang_config.effective_max_queue_depth();
    settings
}

/// Make one call on the router's object.
async fn call<A, R>(member: &str, arguments: A) -> Result<R, RuntimeCallError>
where
    A: serde::Serialize,
    R: serde::de::DeserializeOwned,
{
    let record = registry::find(MODULE_ID)
        .ok_or_else(|| RuntimeCallError::Unavailable(format!("unknown module '{MODULE_ID}'")))?;
    let runtime = host::runtime()
        .await
        .map_err(|_| RuntimeCallError::Unavailable("the module bus is not running".to_string()))?;
    let proxy = runtime
        .proxy(record.bus_name, record.object_path)
        .map_err(|error| RuntimeCallError::Failed(error.to_string()))?;

    proxy
        .call(member, arguments)
        .await
        .map_err(|error| classify(&error))
}

/// Map a bus failure onto the shape a caller can act on.
///
/// The router's own errors arrive as messages rather than distinct wire names,
/// so the classification is on the text it produces — which is stable, because
/// those messages are the module's public contract with a host that renders
/// them. An unrecognised failure is [`RuntimeCallError::Failed`] rather than
/// `InvalidRequest`: telling a caller its request was wrong when it was not
/// sends it into a pointless rewrite.
fn classify(error: &tinybus::Error) -> RuntimeCallError {
    let message = error.to_string();
    if message.contains("ModuleUnavailable") || error.wire_name().contains("ModuleUnavailable") {
        return RuntimeCallError::Unavailable(message);
    }
    if message.contains("no runtime provider is registered")
        || message.contains("named no language")
        || message.contains("is disabled")
    {
        return RuntimeCallError::InvalidRequest(message);
    }
    RuntimeCallError::Failed(message)
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;

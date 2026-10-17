//! The process-wide [`SkillRegistry`], configured from the environment.
//!
//! | Variable | Effect |
//! | --- | --- |
//! | `OPENHUMAN_SKILL_REGISTRY_CATALOG_URL` | Read the Hermes index from this URL instead of the public one |
//! | `OPENHUMAN_SKILL_REGISTRY_DOWNLOAD_BASE_URL` | Serve every `SKILL.md` from `<base>/<name>/SKILL.md` |
//! | `OPENHUMAN_SKILL_INSTALL_ALLOW_LOCAL_HTTP=1` | Allow plain `http` to loopback (fixtures, local mirrors) |
//! | `OPENHUMAN_SKILL_REGISTRY_CACHE_DIR` | Cache directory (legacy catalog files to import; default `~/.openhuman/skill-registry`) |
//!
//! A failed catalog refresh is not attempted again for [`REFRESH_COOLDOWN`]
//! (or longer when the upstream sends `Retry-After`); reads in that window
//! answer from the held catalog or with the last error.
//!
//! The handle is rebuilt whenever that configuration changes, so a process
//! that re-points the environment (tests, a relocated home) gets a registry
//! for its current settings.

use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use tinyskills::{FetchPolicy, HermesIndexSource, RegistryLimits, RegistryTimeouts, SkillRegistry};

use super::store::HostCatalogStore;
use super::transport::ReqwestTransport;

const CATALOG_URL_ENV: &str = "OPENHUMAN_SKILL_REGISTRY_CATALOG_URL";
pub(crate) const DOWNLOAD_BASE_URL_ENV: &str = "OPENHUMAN_SKILL_REGISTRY_DOWNLOAD_BASE_URL";
const CACHE_DIR_ENV: &str = "OPENHUMAN_SKILL_REGISTRY_CACHE_DIR";
const DEFAULT_CACHE_DIR: &str = "skill-registry";
const LEGACY_CACHE_FILE: &str = "cache.json";

/// Minimum wait after a failed catalog refresh before the next one.
pub const REFRESH_COOLDOWN: Duration = Duration::from_secs(45);

/// The registry id of the Hermes index source.
pub const HERMES_REGISTRY_ID: &str = "hermes";

/// Everything the registry is built from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RegistryConfig {
    pub(crate) catalog_url: Option<String>,
    pub(crate) download_base: Option<String>,
    pub(crate) allow_loopback_http: bool,
    pub(crate) cache_dir: Option<PathBuf>,
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

impl RegistryConfig {
    pub(crate) fn from_env() -> Self {
        Self::from_lookup(|name| std::env::var(name).ok(), dirs::home_dir())
    }

    pub(crate) fn from_lookup(
        lookup: impl Fn(&str) -> Option<String>,
        home: Option<PathBuf>,
    ) -> Self {
        let cache_dir = non_empty(lookup(CACHE_DIR_ENV))
            .map(PathBuf::from)
            .or_else(|| home.map(|home| home.join(".openhuman").join(DEFAULT_CACHE_DIR)));
        Self {
            catalog_url: non_empty(lookup(CATALOG_URL_ENV)),
            download_base: non_empty(lookup(DOWNLOAD_BASE_URL_ENV)),
            allow_loopback_http: crate::skills::ops_install::allow_local_http(lookup(
                crate::skills::ops_install::ALLOW_LOCAL_HTTP_ENV,
            )),
            cache_dir,
        }
    }

    pub(crate) fn source(&self) -> HermesIndexSource {
        let source = match &self.catalog_url {
            Some(url) => HermesIndexSource::new(HERMES_REGISTRY_ID, url.clone())
                .with_label("Hermes Skills Hub"),
            None => HermesIndexSource::hermes(),
        };
        match &self.download_base {
            Some(base) => source.with_download_base(base.clone()),
            None => source,
        }
    }

    pub(crate) fn policy(&self) -> FetchPolicy {
        let mut policy = FetchPolicy::default();
        policy.allow_loopback_http = self.allow_loopback_http;
        policy.user_agent = format!("openhuman-core/{}", env!("CARGO_PKG_VERSION"));
        policy
    }

    pub(crate) fn limits() -> RegistryLimits {
        let mut limits = RegistryLimits::default();
        limits.max_page_size = limits.max_entries;
        limits
    }

    pub(crate) fn timeouts() -> RegistryTimeouts {
        let mut timeouts = RegistryTimeouts::default();
        timeouts.cooldown = REFRESH_COOLDOWN;
        timeouts
    }

    fn build(&self) -> Arc<SkillRegistry> {
        let builder = SkillRegistry::builder(ReqwestTransport::new())
            .source(self.source())
            .policy(self.policy())
            .timeouts(Self::timeouts())
            .limits(Self::limits());
        match &self.cache_dir {
            Some(dir) => {
                remove_legacy_cache(dir);
                builder.store(HostCatalogStore::new(dir.clone())).build()
            }
            None => {
                tracing::warn!("[skill_registry] no home directory; catalog cached in memory only");
                builder.build()
            }
        }
    }
}

fn remove_legacy_cache(dir: &Path) {
    let legacy = dir.join(LEGACY_CACHE_FILE);
    match std::fs::remove_file(&legacy) {
        Ok(()) => tracing::info!(
            path = %legacy.display(),
            "[skill_registry] removed legacy catalog cache"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => tracing::warn!(
            path = %legacy.display(),
            error = %error,
            "[skill_registry] could not remove legacy catalog cache"
        ),
    }
}

/// The time budgets every registry fetch in this process uses.
pub(crate) fn registry_timeouts() -> RegistryTimeouts {
    RegistryConfig::timeouts()
}

static HANDLE: RwLock<Option<(RegistryConfig, Arc<SkillRegistry>)>> = RwLock::new(None);

/// The registry for the current environment, built on first use.
pub fn skill_registry() -> Arc<SkillRegistry> {
    registry_for(RegistryConfig::from_env())
}

pub(crate) fn registry_for(config: RegistryConfig) -> Arc<SkillRegistry> {
    if let Some((held, registry)) = HANDLE
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
    {
        if *held == config {
            return Arc::clone(registry);
        }
    }
    let mut handle = HANDLE.write().unwrap_or_else(PoisonError::into_inner);
    if let Some((held, registry)) = handle.as_ref() {
        if *held == config {
            return Arc::clone(registry);
        }
    }
    tracing::info!(
        custom_catalog = config.catalog_url.is_some(),
        download_base = config.download_base.is_some(),
        allow_loopback_http = config.allow_loopback_http,
        persisted = config.cache_dir.is_some(),
        "[skill_registry] building registry"
    );
    let registry = config.build();
    *handle = Some((config, Arc::clone(&registry)));
    registry
}

#[cfg(test)]
pub(crate) fn reset_for_tests() {
    *HANDLE.write().unwrap_or_else(PoisonError::into_inner) = None;
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;

//! Host adapter for the local jail's filesystem grants.
//!
//! TinyBox owns grant discovery (`ResolveLocalGrants`): toolchain homes, git
//! config includes, path normalisation and the credential-ancestor check. The
//! host owns the policy: it sends its protected-path floor
//! (`SecurityPolicy::is_always_forbidden`) with the request and re-applies that
//! same floor to whatever comes back, so a module that returned a forbidden
//! path would still not widen the jail.

use std::path::PathBuf;

use super::types::SandboxPolicy;
use crate::config::{Config, LocalJailConfig};
use crate::security::SecurityPolicy;

/// Credential directories under `$HOME` that no grant may reach or contain.
const HOME_CREDENTIAL_DIRS: &[&str] = &[".ssh", ".gnupg", ".aws", ".azure", ".kube"];

/// Paths the jail is granted beyond the workspace root.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct JailGrants {
    pub read_only: Vec<PathBuf>,
    pub read_write: Vec<PathBuf>,
}

/// The operator home a grant set may be built from. A tenant never reads the
/// operator's home, so SaaS callers get none and `home` is not consulted.
pub(crate) fn grant_home(saas: bool, home: impl FnOnce() -> Option<PathBuf>) -> Option<PathBuf> {
    if saas {
        None
    } else {
        home()
    }
}

/// Add the TinyBox-resolved toolchain grants to a local policy.
///
/// Explicit mounts already on the policy are kept. Without the `modules`
/// feature there is no jail to grant for, so the policy is returned as is.
pub(crate) async fn with_resolved_grants(
    config: &Config,
    policy: &SandboxPolicy,
) -> anyhow::Result<SandboxPolicy> {
    let mut policy = policy.clone();
    #[cfg(feature = "modules")]
    {
        let home = grant_home(crate::core::runtime::is_saas(), dirs::home_dir);
        let grants = resolve(config, home.as_deref(), &config.runtime.local_jail).await?;
        tracing::debug!(
            read_only = grants.read_only.len(),
            read_write = grants.read_write.len(),
            "[sandbox:grants] resolved local jail grants"
        );
        merge(&mut policy.read_only_mounts, grants.read_only);
        merge(&mut policy.read_write_mounts, grants.read_write);
    }
    #[cfg(not(feature = "modules"))]
    let _ = config;
    Ok(policy)
}

#[cfg(feature = "modules")]
fn merge(into: &mut Vec<PathBuf>, extra: Vec<PathBuf>) {
    for path in extra {
        if !into.contains(&path) {
            into.push(path);
        }
    }
}

#[cfg(feature = "modules")]
async fn resolve(
    config: &Config,
    home: Option<&std::path::Path>,
    jail: &LocalJailConfig,
) -> anyhow::Result<JailGrants> {
    let request = grant_request(home, jail);
    let reply = crate::modules::tinybox::resolve_local_grants(config, request)
        .await
        .map_err(|error| anyhow::anyhow!("TinyBox could not resolve local jail grants: {error}"))?;
    Ok(admit_module_grants(reply, jail.allow_proc))
}

/// Build the `ResolveLocalGrants` request: the operator's grant configuration
/// plus the host's protected-path floor.
#[cfg(feature = "modules")]
pub(crate) fn grant_request(
    home: Option<&std::path::Path>,
    jail: &LocalJailConfig,
) -> crate::modules::tinybox::contract::ResolveLocalGrantsRequest {
    use crate::modules::tinybox::contract::{
        LocalGrantConfig, ProtectedPaths, ResolveLocalGrantsRequest,
    };
    use crate::security::policy::{
        ALWAYS_FORBIDDEN_PREFIXES, SENSITIVE_COMPONENTS, SENSITIVE_FILES, SENSITIVE_PAIRS,
    };

    let owned = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    ResolveLocalGrantsRequest {
        home: home.map(|h| h.display().to_string()),
        config: LocalGrantConfig {
            toolchain_homes: jail.toolchain_homes,
            extra_read_only: jail.extra_read_only.clone(),
            extra_read_write: jail.extra_read_write.clone(),
            allow_proc: jail.allow_proc,
        },
        protected: ProtectedPaths {
            components: owned(SENSITIVE_COMPONENTS)
                .into_iter()
                .chain(owned(SENSITIVE_FILES))
                .collect(),
            component_pairs: SENSITIVE_PAIRS
                .iter()
                .map(|[a, b]| [(*a).to_owned(), (*b).to_owned()])
                .collect(),
            prefixes: owned(ALWAYS_FORBIDDEN_PREFIXES),
            credential_roots: home
                .map(|h| {
                    HOME_CREDENTIAL_DIRS
                        .iter()
                        .map(|dir| h.join(dir).display().to_string())
                        .collect()
                })
                .unwrap_or_default(),
        },
    }
}

/// Second gate: drop every returned path the host floor forbids. `/proc` is
/// the one sanctioned exception, and only when `allow_proc` is set.
#[cfg(feature = "modules")]
pub(crate) fn admit_module_grants(
    reply: crate::modules::tinybox::contract::LocalGrants,
    allow_proc: bool,
) -> JailGrants {
    let admit = |raw: String| -> Option<PathBuf> {
        let path = PathBuf::from(raw);
        let sanctioned_proc = allow_proc && path == std::path::Path::new("/proc");
        if !sanctioned_proc && SecurityPolicy::is_always_forbidden(&path) {
            tracing::warn!(
                path = %path.display(),
                "[sandbox:grants] refused module grant: host floor forbids it"
            );
            return None;
        }
        Some(path)
    };
    JailGrants {
        read_only: reply.read_only.into_iter().filter_map(admit).collect(),
        read_write: reply.read_write.into_iter().filter_map(admit).collect(),
    }
}

#[cfg(test)]
#[path = "local_grants_tests.rs"]
mod tests;

//! Host adapter for the local jail's filesystem grants.
//!
//! TinyBox owns grant discovery (`ResolveLocalGrants`): toolchain homes, git
//! config includes, path normalisation and the credential-ancestor check. The
//! host owns the policy: it sends the protected-path floor it asked TinySecurity
//! for (`ProtectedFloor2`) with the request and re-applies that same floor
//! (`ProtectedPath2`) to whatever comes back, so a module that returned a
//! forbidden path would still not widen the jail. The host keeps no list of its own.

use std::path::PathBuf;

use super::types::SandboxPolicy;
use crate::config::Config;
#[cfg(feature = "modules")]
use crate::config::LocalJailConfig;

/// Credential directories under `$HOME` that no grant may reach or contain.
const HOME_CREDENTIAL_DIRS: &[&str] = &[".ssh", ".gnupg", ".aws", ".azure", ".kube"];

/// Absolute credential-store paths under `home` the host floor protects: the
/// `deny_read` list for a jail whose reads are allow-by-default. TinyBox drops
/// the ones that do not exist.
pub(crate) fn credential_store_paths(home: Option<&std::path::Path>) -> Vec<String> {
    let Some(home) = home else {
        return Vec::new();
    };
    let mut paths: Vec<String> = HOME_CREDENTIAL_DIRS
        .iter()
        .map(|dir| home.join(dir).display().to_string())
        .collect();
    if cfg!(target_os = "macos") {
        paths.push(home.join("Library").join("Keychains").display().to_string());
    }
    paths
}

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
    #[cfg(feature = "modules")]
    let mut policy = policy.clone();
    #[cfg(not(feature = "modules"))]
    let policy = policy.clone();
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
    let request = grant_request(home, jail).await.map_err(|error| {
        anyhow::anyhow!("TinySecurity could not supply the path floor: {error}")
    })?;
    let reply = crate::modules::tinybox::resolve_local_grants(config, request)
        .await
        .map_err(|error| anyhow::anyhow!("TinyBox could not resolve local jail grants: {error}"))?;
    Ok(admit_module_grants(reply, jail.allow_proc).await)
}

/// Build the `ResolveLocalGrants` request: the operator's grant configuration
/// plus the protected-path floor TinySecurity owns. `Err` when the module cannot
/// supply the floor: the request is then not sent, so a jail is never granted
/// against a floor the host had to guess.
#[cfg(feature = "modules")]
pub(crate) async fn grant_request(
    home: Option<&std::path::Path>,
    jail: &LocalJailConfig,
) -> Result<crate::modules::tinybox::contract::ResolveLocalGrantsRequest, String> {
    use crate::modules::tinybox::contract::{
        LocalGrantConfig, ProtectedPaths, ResolveLocalGrantsRequest,
    };

    let floor = crate::security::floor::floor().await?;
    Ok(ResolveLocalGrantsRequest {
        home: home.map(|h| h.display().to_string()),
        config: LocalGrantConfig {
            toolchain_homes: jail.toolchain_homes,
            extra_read_only: jail.extra_read_only.clone(),
            extra_read_write: jail.extra_read_write.clone(),
            allow_proc: jail.allow_proc,
        },
        protected: ProtectedPaths {
            components: floor.components.into_iter().chain(floor.files).collect(),
            component_pairs: floor.pairs,
            prefixes: floor.prefixes,
            credential_roots: home
                .map(|h| {
                    HOME_CREDENTIAL_DIRS
                        .iter()
                        .map(|dir| h.join(dir).display().to_string())
                        .collect()
                })
                .unwrap_or_default(),
        },
    })
}

/// Second gate: drop every returned path the floor forbids. `/proc` is the one
/// sanctioned exception, and only when `allow_proc` is set. One module call covers
/// the whole reply; if it cannot answer, nothing is admitted.
#[cfg(feature = "modules")]
pub(crate) async fn admit_module_grants(
    reply: crate::modules::tinybox::contract::LocalGrants,
    allow_proc: bool,
) -> JailGrants {
    let read_only: Vec<PathBuf> = reply.read_only.into_iter().map(PathBuf::from).collect();
    let read_write: Vec<PathBuf> = reply.read_write.into_iter().map(PathBuf::from).collect();
    let all: Vec<PathBuf> = read_only.iter().chain(&read_write).cloned().collect();
    let protected = match crate::security::floor::protected_paths(&all).await {
        Ok(answers) if answers.len() == all.len() => answers,
        other => {
            tracing::warn!(
                error = ?other.err(),
                "[sandbox:grants] floor unavailable; no module grant admitted"
            );
            return JailGrants::default();
        }
    };
    let mut verdict = all.iter().zip(protected);
    let mut admit = |paths: Vec<PathBuf>| -> Vec<PathBuf> {
        paths
            .into_iter()
            .filter(|path| {
                let (_, forbidden) = verdict.next().expect("one verdict per path");
                let sanctioned_proc = allow_proc && path == std::path::Path::new("/proc");
                if forbidden && !sanctioned_proc {
                    tracing::warn!(
                        path = %path.display(),
                        "[sandbox:grants] refused module grant: the floor forbids it"
                    );
                    return false;
                }
                true
            })
            .collect()
    };
    let read_only = admit(read_only);
    let read_write = admit(read_write);
    JailGrants {
        read_only,
        read_write,
    }
}

#[cfg(test)]
#[path = "local_grants_tests.rs"]
mod tests;

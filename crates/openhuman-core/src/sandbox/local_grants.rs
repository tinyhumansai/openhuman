//! Host adapter for the local jail's filesystem grants.
//!
//! TinyBox owns grant discovery (`ResolveLocalGrants`): toolchain homes, git
//! config includes, path normalisation and the credential-ancestor check. The
//! host owns the policy context: TinySecurity supplies its protected-path floor
//! and credential roots, which OpenHuman re-applies to returned grants before
//! passing them to TinyBox.

use std::path::PathBuf;

use super::types::SandboxPolicy;
use crate::config::Config;
#[cfg(all(feature = "modules", feature = "security-module"))]
use crate::config::LocalJailConfig;

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
    #[cfg(all(feature = "modules", feature = "security-module"))]
    let mut policy = policy.clone();
    #[cfg(not(all(feature = "modules", feature = "security-module")))]
    let policy = policy.clone();
    #[cfg(all(feature = "modules", feature = "security-module"))]
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
    #[cfg(not(all(feature = "modules", feature = "security-module")))]
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

#[cfg(all(feature = "modules", feature = "security-module"))]
async fn resolve(
    config: &Config,
    home: Option<&std::path::Path>,
    jail: &LocalJailConfig,
) -> anyhow::Result<JailGrants> {
    let request = grant_request(home, jail).await.map_err(|error| {
        anyhow::anyhow!("TinySecurity could not provide the local jail path floor: {error}")
    })?;
    let reply = crate::modules::tinybox::resolve_local_grants(config, request)
        .await
        .map_err(|error| anyhow::anyhow!("TinyBox could not resolve local jail grants: {error}"))?;
    Ok(JailGrants {
        read_only: reply.read_only.into_iter().map(PathBuf::from).collect(),
        read_write: reply.read_write.into_iter().map(PathBuf::from).collect(),
    })
}

/// Build the `ResolveLocalGrants` request from TinySecurity's floor vocabulary.
#[cfg(all(feature = "modules", feature = "security-module"))]
pub(crate) async fn grant_request(
    home: Option<&std::path::Path>,
    jail: &LocalJailConfig,
) -> Result<crate::modules::tinybox::contract::ResolveLocalGrantsRequest, String> {
    use crate::modules::tinybox::contract::{
        LocalGrantConfig, ProtectedPaths, ResolveLocalGrantsRequest,
    };
    let floor = crate::modules::security::gap::protected_floor().await?;
    let credential_roots =
        crate::modules::security::gap::credential_roots(home.map(std::path::Path::to_path_buf))
            .await?;
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
            credential_roots: credential_roots
                .into_iter()
                .map(|path| path.display().to_string())
                .collect(),
        },
    })
}

#[cfg(all(test, feature = "modules", feature = "security-module"))]
#[path = "local_grants_tests.rs"]
mod tests;

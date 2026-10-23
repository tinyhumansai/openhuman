//! The jail OpenHuman grants stdio MCP servers.
//!
//! TinyMCP starts a stdio server inside TinyBox's native jail when the host
//! gives it a [`StdioSandboxConfig`]; without one (and without the
//! operator-trusted flag on the server) it refuses to start the server. This
//! module is the host policy for that config: where the server's writable
//! workspace lives, what it may read, which credential stores it can never
//! reach, and that the backend must meet the untrusted-code isolation floor.
//!
//! Grants follow `[runtime.local_jail]`, the same operator policy the shell
//! sandbox uses, so one setting widens or narrows both. TinyBox owns the
//! discovery of toolchain homes (`ResolveLocalGrants`); [`prepare`] asks for it
//! once at boot and [`for_config`] serves the answer synchronously. Until it is
//! prepared the jail is the system baseline plus the operator's extras, which
//! is narrower, never wider.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use tinymcp_bus::{StdioSandboxConfig, StdioSandboxEnforcement};

use crate::config::Config;
use crate::sandbox::local_grants::{
    admit_module_grants, credential_store_paths, grant_home, grant_request, JailGrants,
};

/// System paths a server's runtime commonly needs, read-only. Missing ones
/// are skipped, since TinyBox rejects a grant it cannot canonicalize.
const SYSTEM_BASELINE: &[&str] = &[
    "/usr", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/etc", "/opt", "/nix",
];

type Prepared = Mutex<HashMap<PathBuf, JailGrants>>;

fn prepared() -> &'static Prepared {
    static PREPARED: OnceLock<Prepared> = OnceLock::new();
    PREPARED.get_or_init(Mutex::default)
}

/// The server's writable root: a directory of its own under the core's
/// workspace, never the workspace itself.
pub(crate) fn workspace(config: &Config) -> PathBuf {
    config.workspace_dir.join("mcp").join("stdio")
}

fn existing(paths: impl IntoIterator<Item = String>) -> Vec<String> {
    paths
        .into_iter()
        .filter(|path| Path::new(path).exists())
        .collect()
}

/// The sandbox for `config`, or `None` when the workspace cannot be created
/// (the servers are then refused rather than started without a jail).
pub(crate) fn for_config(config: &Config) -> Option<StdioSandboxConfig> {
    let workspace = workspace(config);
    if let Err(error) = std::fs::create_dir_all(&workspace) {
        tracing::warn!(
            error = %error,
            "[mcp:sandbox] cannot create the stdio workspace; stdio servers will be refused"
        );
        return None;
    }
    let jail = &config.runtime.local_jail;
    let discovered = prepared()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&config.workspace_dir)
        .cloned()
        .unwrap_or_default();
    let mut read_only: Vec<String> = SYSTEM_BASELINE
        .iter()
        .map(|path| (*path).to_owned())
        .chain(jail.extra_read_only.iter().cloned())
        .chain(
            discovered
                .read_only
                .iter()
                .map(|path| path.display().to_string()),
        )
        .collect();
    let mut read_write: Vec<String> = jail
        .extra_read_write
        .iter()
        .cloned()
        .chain(
            discovered
                .read_write
                .iter()
                .map(|path| path.display().to_string()),
        )
        .collect();
    read_only = existing(read_only);
    read_write = existing(read_write);
    dedup(&mut read_only);
    dedup(&mut read_write);
    let home = grant_home(crate::core::runtime::is_saas(), dirs::home_dir);
    Some(StdioSandboxConfig {
        workspace: workspace.display().to_string(),
        read_only,
        read_write,
        deny_read: credential_store_paths(home.as_deref()),
        allow_net: true,
        allow_subprocess: true,
        // Third-party code: the backend must meet the untrusted-code floor or
        // the server is refused.
        require_process_isolation: true,
        enforcement: StdioSandboxEnforcement::Enforced,
    })
}

fn dedup(paths: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    paths.retain(|path| seen.insert(path.clone()));
}

/// Asks TinyBox for the toolchain grants and remembers them for
/// [`for_config`]. A failure is logged and leaves the narrower baseline.
pub(crate) async fn prepare(config: &Config) {
    let home = grant_home(crate::core::runtime::is_saas(), dirs::home_dir);
    let request = match grant_request(home.as_deref(), &config.runtime.local_jail).await {
        Ok(request) => request,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "[mcp:sandbox] no protected-path floor; using the system baseline"
            );
            return;
        }
    };
    match crate::modules::tinybox::resolve_local_grants(config, request).await {
        Ok(reply) => {
            let grants = admit_module_grants(reply, config.runtime.local_jail.allow_proc).await;
            tracing::debug!(
                read_only = grants.read_only.len(),
                read_write = grants.read_write.len(),
                "[mcp:sandbox] resolved stdio server grants"
            );
            prepared()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(config.workspace_dir.clone(), grants);
        }
        Err(error) => tracing::warn!(
            error = %error,
            "[mcp:sandbox] could not resolve toolchain grants; using the system baseline"
        ),
    }
}

#[cfg(test)]
#[path = "stdio_sandbox_tests.rs"]
mod tests;

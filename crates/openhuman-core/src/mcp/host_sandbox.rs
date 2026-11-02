//! The jail OpenHuman grants stdio MCP servers.
//!
//! A stdio MCP server is third-party code that runs with the user's
//! credentials in its environment. The `tinymcp` contract
//! (`McpClientConfig::stdio_sandbox`, `docs/specs/stdio-sandbox.md` in the
//! module) starts every server that is not `operator_trusted` inside a
//! TinyBox jail built from this config, and refuses it when none is given.
//! Registry-installed and `mcp.json`-declared servers are never
//! operator-trusted, so the host must supply the jail; this module builds it
//! from the same policy the shell tool's native jail uses:
//!
//! - the workspace is a dedicated writable directory under the core's
//!   `workspace_dir`;
//! - read and write grants come from `[runtime.local_jail]` plus the
//!   TinyBox-resolved toolchain grants (`sandbox::local_grants`), each passed
//!   through the host's protected-path floor;
//! - the untrusted-code isolation floor stays on, except on the macOS desktop
//!   where Seatbelt folder confinement is the host's accepted jail
//!   (`sandbox::native_jail::jail_isolation`), and there the credential stores
//!   are denied by path because Seatbelt reads allow-by-default.

use std::path::{Path, PathBuf};

use tinymcp_bus::{StdioSandboxConfig, StdioSandboxEnforcement};

use crate::config::Config;

/// Directory under `workspace_dir` that is the stdio servers' writable root.
const WORKSPACE_DIR: &str = "mcp_stdio";

/// Builds the jail for `config`, or `None` when the host cannot offer one
/// (no TinyBox support in this build, or the workspace cannot be created).
/// `None` makes the module refuse stdio servers rather than start them
/// unconfined.
#[cfg(feature = "modules")]
pub(super) async fn stdio_sandbox(config: &Config) -> Option<StdioSandboxConfig> {
    let workspace = match prepare_workspace(&config.workspace_dir) {
        Ok(path) => path,
        Err(error) => {
            tracing::warn!(%error, "[mcp] stdio sandbox workspace unavailable; stdio servers will be refused");
            return None;
        }
    };
    let saas = crate::core::runtime::is_saas();
    let home = crate::sandbox::local_grants::grant_home(saas, dirs::home_dir);
    let jail = &config.runtime.local_jail;
    let isolated = require_process_isolation(saas);
    let deny_read = if isolated {
        Vec::new()
    } else {
        match credential_denials(config, home.as_deref(), &workspace).await {
            Ok(denied) => denied,
            Err(error) => {
                tracing::warn!(%error, "[mcp] TinySecurity credential roots unavailable; stdio servers will be refused");
                return None;
            }
        }
    };
    let read_only = admit_all(&jail.extra_read_only, home.as_deref()).await;
    let read_write = admit_all(&jail.extra_read_write, home.as_deref()).await;
    Some(StdioSandboxConfig {
        workspace: path_string(&workspace),
        read_only,
        read_write,
        deny_read,
        // MCP servers commonly call remote APIs and `npx` starts children.
        allow_net: true,
        allow_subprocess: true,
        require_process_isolation: isolated,
        enforcement: enforcement(isolated),
    })
}

#[cfg(not(feature = "modules"))]
pub(super) async fn stdio_sandbox(_config: &Config) -> Option<StdioSandboxConfig> {
    None
}

/// Adds the TinyBox-resolved toolchain grants (the runtimes `npx`, `uvx` and
/// friends live in) to `sandbox`. A resolution failure keeps the config
/// grants only: the jail never widens on error.
#[cfg(feature = "modules")]
pub(super) async fn with_toolchain_grants(config: &Config, sandbox: &mut StdioSandboxConfig) {
    use crate::sandbox::types::SandboxPolicy;
    let seed = SandboxPolicy {
        backend: crate::sandbox::types::SandboxBackendKind::Local,
        workspace_root: PathBuf::from(&sandbox.workspace),
        state_dir: config.workspace_dir.clone(),
        read_only_mounts: Vec::new(),
        read_write_mounts: Vec::new(),
        allow_network: sandbox.allow_net,
        mount_workspace: true,
        env_passthrough: Vec::new(),
        docker_overrides: None,
    };
    match crate::sandbox::local_grants::with_resolved_grants(config, &seed).await {
        Ok(resolved) => {
            merge(&mut sandbox.read_only, &resolved.read_only_mounts);
            merge(&mut sandbox.read_write, &resolved.read_write_mounts);
            tracing::debug!(
                read_only = sandbox.read_only.len(),
                read_write = sandbox.read_write.len(),
                "[mcp] stdio sandbox grants resolved"
            );
        }
        Err(error) => tracing::warn!(
            %error,
            "[mcp] toolchain grants unavailable; stdio sandbox keeps configured grants only"
        ),
    }
}

#[cfg(not(feature = "modules"))]
pub(super) async fn with_toolchain_grants(_config: &Config, _sandbox: &mut StdioSandboxConfig) {}

/// Whether the backend must meet the untrusted-code isolation floor. The
/// macOS desktop sandbox (Seatbelt) is accepted as folder confinement for the
/// local user, exactly as the shell tool's jail accepts it; a SaaS tenant
/// always requires the floor.
pub(super) fn require_process_isolation(saas: bool) -> bool {
    saas || !cfg!(target_os = "macos")
}

/// How the module treats a backend that cannot meet the isolation floor
/// (`StdioSandboxEnforcement`, tinymcp contract 1.27). Where the host accepts a
/// best-effort backend (the macOS desktop's Seatbelt) the jail is `Declared`,
/// so the module starts the server instead of refusing the best-effort
/// backend; everywhere else it is `Enforced`.
pub(super) fn enforcement(isolated: bool) -> StdioSandboxEnforcement {
    if isolated {
        StdioSandboxEnforcement::Enforced
    } else {
        StdioSandboxEnforcement::Declared
    }
}

fn merge(into: &mut Vec<String>, extra: &[PathBuf]) {
    for path in extra {
        let path = path_string(path);
        if !into.contains(&path) {
            into.push(path);
        }
    }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn prepare_workspace(workspace_dir: &Path) -> std::io::Result<PathBuf> {
    let dir = workspace_dir.join(WORKSPACE_DIR);
    std::fs::create_dir_all(&dir)?;
    dir.canonicalize()
}

/// Existing absolute grants from configuration, `~/` expanded, each past the
/// host's protected-path floor and the credential-ancestor check.
#[cfg(feature = "security-module")]
async fn admit_all(raw: &[String], home: Option<&Path>) -> Vec<String> {
    let credential_roots = match crate::modules::security::gap::credential_roots(
        home.map(Path::to_path_buf),
    )
    .await
    {
        Ok(roots) => roots,
        Err(error) => {
            tracing::warn!(%error, "[mcp] TinySecurity credential roots unavailable; refusing configured jail grants");
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    let mut candidates = Vec::new();
    for entry in raw {
        let expanded = match (entry.strip_prefix("~/"), home) {
            (Some(rest), Some(home)) => home.join(rest),
            _ => PathBuf::from(entry),
        };
        if !expanded.is_absolute() || !expanded.exists() {
            continue;
        }
        let path = expanded.canonicalize().unwrap_or_else(|_| expanded.clone());
        candidates.push((expanded, path));
    }
    let paths = candidates
        .iter()
        .flat_map(|(spelled, resolved)| [spelled.clone(), resolved.clone()])
        .collect::<Vec<_>>();
    let protected = match crate::modules::security::gap::protected_paths(&paths).await {
        Ok(answers) if answers.len() == paths.len() => answers,
        Ok(_) => {
            tracing::warn!(
                "[mcp] TinySecurity protected-path response was malformed; refusing jail grants"
            );
            return Vec::new();
        }
        Err(error) => {
            tracing::warn!(%error, "[mcp] TinySecurity protected-path check unavailable; refusing jail grants");
            return Vec::new();
        }
    };
    for (index, (spelled, resolved)) in candidates.into_iter().enumerate() {
        if protected[index * 2]
            || protected[index * 2 + 1]
            || credential_roots.iter().any(|root| {
                root.starts_with(&spelled)
                    || root.starts_with(&resolved)
                    || spelled.starts_with(root)
                    || resolved.starts_with(root)
            })
        {
            tracing::warn!(
                "[mcp] refused a stdio sandbox grant: TinySecurity path floor forbids it"
            );
            continue;
        }
        let path = path_string(&resolved);
        if !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

#[cfg(not(feature = "security-module"))]
async fn admit_all(_raw: &[String], _home: Option<&Path>) -> Vec<String> {
    // Without TinySecurity there is no authority to admit extra path grants.
    Vec::new()
}

#[cfg(all(feature = "modules", feature = "security-module"))]
async fn credential_denials(
    config: &Config,
    home: Option<&Path>,
    workspace: &Path,
) -> Result<Vec<String>, String> {
    let stores = crate::modules::security::gap::credential_roots(home.map(Path::to_path_buf))
        .await?
        .into_iter()
        .chain(crate::credentials::credential_files(config));
    let mut denied = Vec::new();
    for store in stores {
        let Ok(real) = store.canonicalize() else {
            continue;
        };
        if workspace.starts_with(&real) {
            tracing::warn!("[mcp] stdio workspace is inside a credential store; not denying it");
            continue;
        }
        let real = path_string(&real);
        if !denied.contains(&real) {
            denied.push(real);
        }
    }
    Ok(denied)
}

#[cfg(all(feature = "modules", not(feature = "security-module")))]
async fn credential_denials(
    _config: &Config,
    _home: Option<&Path>,
    _workspace: &Path,
) -> Result<Vec<String>, String> {
    Err("TinySecurity is not built into this host".into())
}

#[cfg(all(test, feature = "security-module"))]
#[path = "host_sandbox_tests.rs"]
mod tests;

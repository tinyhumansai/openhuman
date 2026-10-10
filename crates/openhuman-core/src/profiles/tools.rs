//! Which agent tools a user gets in SaaS mode, and where they run.
//!
//! A profile's tool list starts from its domain families
//! ([`host::user_domains`](super::host::user_domains)): memory and thread
//! tools, nothing that reaches the host. Shell and file tools sit in the
//! `Platform` family, which a user context never enables. The operator opts
//! users into them per deployment with `tool_allowlist`, one
//! [`SaasToolGroup`] at a time:
//!
//! - `host_files`: the file tools. They run in-process, confined by the
//!   user's policy (`layout::profile_config`: autonomy on, workspace-only,
//!   `action_dir` = the user's `sandbox/`, no other trusted roots).
//! - `host_shell`: the shell. Every command runs in a fresh container
//!   ([`sandbox_policy`]) whose only writable mount is the user's `sandbox/`.
//!   If the container cannot start, the command fails; it never falls back to
//!   the host.
//!
//! Some tools stay out whatever the allowlist says ([`HARD_DENIED`]): they
//! change the process, install code, or reach state shared by every user.
//!
//! There is no per-user approval surface yet, so in SaaS the approval gate
//! does not park: it allows a tool from an allowlisted group, whose reach the
//! sandbox and path policy already bound, and refuses anything else
//! ([`gate_verdict`]).

use std::path::Path;

use crate::core::runtime::is_saas;
use crate::core::runtime::saas::SaasSandboxConfig;
use crate::sandbox::{DockerOverrides, SandboxBackendKind, SandboxPolicy};

/// A group of host tools an operator can opt users into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaasToolGroup {
    HostFiles,
    HostShell,
}

impl SaasToolGroup {
    pub const ALL: [Self; 2] = [Self::HostFiles, Self::HostShell];

    /// The group named `id` in `tool_allowlist`.
    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|group| group.id() == id.trim())
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::HostFiles => "host_files",
            Self::HostShell => "host_shell",
        }
    }

    /// The tools the group opens, by `Tool::name`.
    pub fn tools(self) -> &'static [&'static str] {
        match self {
            Self::HostFiles => &[
                "file_read",
                "file_write",
                "edit",
                "apply_patch",
                "grep",
                "glob",
                "list",
                "csv_export",
                "read_workspace_state",
            ],
            Self::HostShell => &["shell"],
        }
    }

    /// Whether the group runs code, and so needs the container sandbox.
    pub fn needs_sandbox(self) -> bool {
        matches!(self, Self::HostShell)
    }

    /// The group owning `tool`, if any.
    pub fn of_tool(tool: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|group| group.tools().contains(&tool))
    }
}

/// Tool name prefixes a user never gets, allowlist or not.
pub const HARD_DENIED_PREFIXES: &[&str] = &[
    "service_",
    "update_",
    "proxy_",
    "mcp_registry_",
    "skill_registry_",
    "hosting_",
    "browser",
    "computer_",
    "desktop_",
    "docker_",
    "process_",
];

/// Tool names a user never gets, allowlist or not.
pub const HARD_DENIED: &[&str] = &[
    "install_tool",
    "install_workflow_from_url",
    "uninstall_workflow",
    "create_skill",
    "git_operations",
    "delegate",
    "curl",
    "pushover",
];

/// Whether `tool` is hard-denied to users.
pub fn is_hard_denied(tool: &str) -> bool {
    HARD_DENIED.contains(&tool)
        || HARD_DENIED_PREFIXES
            .iter()
            .any(|prefix| tool.starts_with(prefix))
}

/// The groups in an operator's `tool_allowlist`, skipping names that are not
/// groups (the boot guard refuses those).
pub fn parse_allowlist(entries: &[String]) -> Vec<SaasToolGroup> {
    entries
        .iter()
        .filter_map(|entry| SaasToolGroup::parse(entry))
        .collect()
}

/// Entries of `tool_allowlist` that name no group.
pub fn unknown_entries(entries: &[String]) -> Vec<String> {
    entries
        .iter()
        .filter(|entry| SaasToolGroup::parse(entry).is_none())
        .cloned()
        .collect()
}

/// Whether a user may have `tool`, given the allowlisted `groups` and whether
/// its domain family is on (`domain_ok`). Hard-denied tools never pass; tools
/// in a host group pass only when that group is allowlisted; any other tool
/// follows its domain family.
pub fn admits_with(tool: &str, domain_ok: bool, groups: &[SaasToolGroup]) -> bool {
    if is_hard_denied(tool) {
        return false;
    }
    match SaasToolGroup::of_tool(tool) {
        Some(group) => groups.contains(&group),
        None => domain_ok,
    }
}

/// The groups the running deployment allowlists; empty outside SaaS.
pub fn allowlisted() -> Vec<SaasToolGroup> {
    super::host::host()
        .map(|host| parse_allowlist(&host.saas().tool_allowlist))
        .unwrap_or_default()
}

/// The tool-list filter. Outside SaaS this is the domain filter alone.
pub fn admits(tool: &str, domain_ok: bool) -> bool {
    if !is_saas() {
        return domain_ok;
    }
    let admitted = admits_with(tool, domain_ok, &allowlisted());
    if !admitted && domain_ok {
        log::debug!("[profiles][tools] withholding `{tool}` from profiles");
    }
    admitted
}

/// The approval gate's answer in SaaS, where it never parks: `Ok` for a tool
/// in an allowlisted group, otherwise the refusal.
pub fn gate_verdict_with(tool: &str, groups: &[SaasToolGroup]) -> Result<(), String> {
    if admits_with(tool, false, groups) {
        return Ok(());
    }
    Err(format!(
        "'{tool}' needs an approval, and this deployment has no approval surface for users"
    ))
}

/// [`gate_verdict_with`] for the running deployment.
pub fn gate_verdict(tool: &str) -> Result<(), String> {
    let verdict = gate_verdict_with(tool, &allowlisted());
    log::debug!(
        "[profiles][tools] approval gate tool={tool} allowed={}",
        verdict.is_ok()
    );
    verdict
}

/// Whether a Docker network name is the host network (Docker matches it
/// case-insensitively).
pub fn is_host_network(network: &str) -> bool {
    network.trim().eq_ignore_ascii_case("host")
}

/// The container policy for a user's shell command. `action_dir` must be a
/// user's `sandbox/` directory under `<root>/users/<profile-id>/`, or the command is
/// refused.
pub fn sandbox_policy_with(
    saas_root: &Path,
    config: &SaasSandboxConfig,
    action_dir: &Path,
    state_dir: &Path,
) -> Result<SandboxPolicy, String> {
    let users = super::layout::users_dir(saas_root);
    let profile_dir = action_dir.parent();
    let is_user_sandbox = profile_dir.and_then(Path::parent) == Some(users.as_path())
        && profile_dir
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .is_some_and(|name| super::types::ProfileId::parse(name).is_ok())
        && action_dir.file_name().is_some_and(|name| name == "sandbox");
    if !is_user_sandbox {
        return Err(format!(
            "{} is not a user sandbox directory",
            action_dir.display()
        ));
    }
    // Resolve symlinks: the directory mounted read-write must really be this
    // agent's sandbox, not a link from it to somewhere else on the host.
    let resolved = action_dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", action_dir.display()))?;
    let expected = users
        .canonicalize()
        .map_err(|e| format!("{}: {e}", users.display()))?
        .join(profile_dir.and_then(Path::file_name).unwrap_or_default())
        .join("sandbox");
    if resolved != expected {
        return Err(format!(
            "{} resolves outside the user's sandbox",
            action_dir.display()
        ));
    }
    if is_host_network(&config.network) {
        return Err("the sandbox cannot use the host network".to_string());
    }
    Ok(SandboxPolicy {
        backend: SandboxBackendKind::Docker,
        workspace_root: resolved,
        state_dir: state_dir.to_path_buf(),
        read_only_mounts: Vec::new(),
        read_write_mounts: Vec::new(),
        allow_network: config.network.trim() != "none",
        // Nothing from the host's environment: no PATH, HOME or USER.
        env_passthrough: Vec::new(),
        docker_overrides: Some(DockerOverrides {
            image: Some(config.image.clone()),
            network: Some(config.network.clone()),
            memory_limit_mb: Some(config.memory_limit_mb),
            cpu_limit: Some(config.cpu_limit),
            read_only_rootfs: Some(true),
            extra_caps_drop: Vec::new(),
        }),
    })
}

/// [`sandbox_policy_with`] for the running deployment.
pub fn sandbox_policy(action_dir: &Path, state_dir: &Path) -> Result<SandboxPolicy, String> {
    let host = super::host::host().ok_or("no profile host is installed")?;
    let saas = host.saas();
    sandbox_policy_with(&saas.root, &saas.sandbox, action_dir, state_dir)
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;

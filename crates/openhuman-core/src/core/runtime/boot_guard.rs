//! The checks a SaaS core must pass before it boots.
//!
//! Fail closed: [`check`] collects **every** violation and refuses the boot
//! if there is one, so an operator fixes a deployment in one pass instead of
//! one error at a time. It is a pure function of [`BootInputs`] — the
//! environment, home directory and token file are read by the caller — so
//! each rule is testable without touching process state.

use std::path::{Path, PathBuf};

use super::saas::SaasConfig;
use super::{DomainSet, ServiceSet};
use crate::core::types::HostKind;

/// Shortest gateway bearer accepted, in bytes.
pub const MIN_SERVICE_TOKEN_LEN: usize = 32;

/// Environment variables that must not be set for a SaaS core, with why.
///
/// Each one either points the core at a single user's state, hands it a
/// process-wide credential, or opens a debugging back door.
pub const FORBIDDEN_ENV: &[(&str, &str)] = &[
    (
        "OPENHUMAN_WORKSPACE",
        "pins one workspace for the whole process",
    ),
    (
        "OPENHUMAN_DEV_CONNECT",
        "exposes the bearer through /dev/connect",
    ),
    (
        "OPENHUMAN_BACKEND_SESSION_TOKEN",
        "a process-wide user session",
    ),
    (
        "OPENHUMAN_CORE_TOKEN",
        "the gateway bearer must come from the service token file",
    ),
];

/// Switches that are fine on but must not turn a protection off.
pub const FORBIDDEN_OFF_SWITCHES: &[(&str, &str)] = &[
    (
        "OPENHUMAN_APPROVAL_GATE",
        "the approval gate cannot be switched off",
    ),
    ("OPENHUMAN_SANDBOX", "sandboxing cannot be switched off"),
];

fn is_off(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "off" | "no" | "none" | "disabled"
    )
}

/// A process-wide backend key; allowed only when the operator opts into
/// [`SaasConfig::shared_backend_api_key`].
pub const SHARED_API_KEY_ENV: &str = "OPENHUMAN_BACKEND_API_KEY";

/// What reading the service token file found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceToken {
    /// A bearer that passed every check.
    Valid(String),
    /// Why the file cannot be used.
    Invalid(String),
}

impl ServiceToken {
    /// Read and vet the token at `path`: present, owner-only, and long enough.
    pub fn read(path: &Path) -> Self {
        use std::io::Read;
        let display = path.display();
        // One open handle for both checks, so the permissions vetted are the
        // permissions of the file whose bytes are read.
        let mut file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) => return Self::Invalid(format!("cannot read {display}: {e}")),
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            match file.metadata() {
                Ok(meta) if meta.permissions().mode() & 0o077 != 0 => {
                    return Self::Invalid(format!(
                        "{display} is readable by others (mode {:o}); chmod 600 it",
                        meta.permissions().mode() & 0o777
                    ));
                }
                Ok(_) => {}
                Err(e) => return Self::Invalid(format!("cannot stat {display}: {e}")),
            }
        }
        let mut raw = String::new();
        if let Err(e) = file.read_to_string(&mut raw) {
            return Self::Invalid(format!("cannot read {display}: {e}"));
        }
        let token = raw.trim();
        // It travels as `Authorization: Bearer <token>`: one line of visible
        // ASCII, or no client could ever send it.
        if !token.bytes().all(|b| b.is_ascii_graphic()) {
            return Self::Invalid(format!(
                "{display} must hold one token of visible ASCII characters (no spaces, \
                 line breaks or control characters)"
            ));
        }
        if token.len() < MIN_SERVICE_TOKEN_LEN {
            return Self::Invalid(format!(
                "{display} holds {} bytes; at least {MIN_SERVICE_TOKEN_LEN} are required",
                token.len()
            ));
        }
        Self::Valid(token.to_string())
    }
}

/// Everything [`check`] decides on.
pub struct BootInputs<'a> {
    pub host_kind: HostKind,
    pub services: ServiceSet,
    pub domains: DomainSet,
    pub config: &'a SaasConfig,
    pub token: &'a ServiceToken,
    pub env: &'a [(String, String)],
    pub home: Option<PathBuf>,
    /// Whether the container sandbox answered (probed only when an
    /// allowlisted tool group needs it).
    pub sandbox_available: bool,
}

/// One reason a SaaS core refuses to boot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    HostKind(HostKind),
    Service(&'static str),
    Domain(&'static str),
    Env { var: String, why: &'static str },
    ToolAllowlist(Vec<String>),
    Sandbox(String),
    RpcAllowlist(Vec<String>),
    Root(String),
    ServiceToken(String),
    Platform(&'static str),
    Cluster(String),
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HostKind(kind) => write!(f, "host kind `{}` cannot serve SaaS", kind.tag()),
            Self::Service(name) => write!(f, "service `{name}` is not allowed in SaaS mode"),
            Self::Domain(name) => write!(f, "domain family `{name}` is not isolated per user yet"),
            Self::Env { var, why } => write!(f, "environment variable {var} is set: {why}"),
            Self::ToolAllowlist(entries) => write!(
                f,
                "tool_allowlist {entries:?}: not tool groups (known: host_files, host_shell)"
            ),
            Self::Sandbox(why) => write!(f, "sandbox: {why}"),
            Self::RpcAllowlist(methods) => write!(
                f,
                "rpc_allowlist_extra {methods:?}: no RPC method can be opted in until the per-user RPC surface ships"
            ),
            Self::Root(why) => write!(f, "root: {why}"),
            Self::ServiceToken(why) => write!(f, "service token: {why}"),
            Self::Platform(why) => write!(f, "platform: {why}"),
            Self::Cluster(why) => write!(f, "cluster: {why}"),
        }
    }
}

/// The boot was refused; every violation found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootGuardError {
    pub violations: Vec<Violation>,
}

impl std::fmt::Display for BootGuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "[saas] refusing to boot: {} problem(s) with this deployment",
            self.violations.len()
        )?;
        for violation in &self.violations {
            writeln!(f, "  - {violation}")?;
        }
        Ok(())
    }
}

impl std::error::Error for BootGuardError {}

/// Refuse a SaaS boot that is not safe to serve users from.
pub fn check(inputs: &BootInputs<'_>) -> Result<(), BootGuardError> {
    let mut violations = Vec::new();

    if !matches!(inputs.host_kind, HostKind::Saas) {
        violations.push(Violation::HostKind(inputs.host_kind));
    }

    let allowed = ServiceSet::saas();
    for ((name, on), (_, ok)) in service_flags(&inputs.services)
        .into_iter()
        .zip(service_flags(&allowed))
    {
        if on && !ok {
            violations.push(Violation::Service(name));
        }
    }
    let allowed = DomainSet::saas();
    for ((name, on), (_, ok)) in domain_flags(&inputs.domains)
        .into_iter()
        .zip(domain_flags(&allowed))
    {
        if on && !ok {
            violations.push(Violation::Domain(name));
        }
    }

    for (var, value) in inputs.env {
        if value.trim().is_empty() {
            continue;
        }
        if let Some((_, why)) = FORBIDDEN_ENV.iter().find(|(name, _)| name == var) {
            violations.push(Violation::Env {
                var: var.clone(),
                why,
            });
        } else if let Some((_, why)) = FORBIDDEN_OFF_SWITCHES
            .iter()
            .find(|(name, _)| name == var && is_off(value))
        {
            violations.push(Violation::Env {
                var: var.clone(),
                why,
            });
        } else if var == SHARED_API_KEY_ENV && !inputs.config.shared_backend_api_key {
            violations.push(Violation::Env {
                var: var.clone(),
                why: "a process-wide backend key; set shared_backend_api_key to allow it",
            });
        }
    }

    let unknown = crate::profiles::tools::unknown_entries(&inputs.config.tool_allowlist);
    if !unknown.is_empty() {
        violations.push(Violation::ToolAllowlist(unknown));
    }
    violations.extend(sandbox_problems(inputs).into_iter().map(Violation::Sandbox));
    if !inputs.config.rpc_allowlist_extra.is_empty() {
        violations.push(Violation::RpcAllowlist(
            inputs.config.rpc_allowlist_extra.clone(),
        ));
    }

    if let Some(why) = root_problem(&inputs.config.root, inputs.home.as_deref()) {
        violations.push(Violation::Root(why));
    }
    // Token and root permissions are verified only on Unix.
    #[cfg(not(unix))]
    violations.push(Violation::Platform(
        "SaaS mode needs a Unix host, where token and root permissions can be verified",
    ));
    if let ServiceToken::Invalid(why) = inputs.token {
        violations.push(Violation::ServiceToken(why.clone()));
    }
    if let Some(why) = cluster_problem(inputs) {
        violations.push(Violation::Cluster(why));
    }

    if violations.is_empty() {
        log::info!("[saas][boot-guard] deployment checks passed");
        Ok(())
    } else {
        for violation in &violations {
            log::error!("[saas][boot-guard] {violation}");
        }
        Err(BootGuardError { violations })
    }
}

/// Why a clustered node (one that sets `advertise_url`) cannot keep its
/// profiles exclusive, if it cannot: its leases need a shared backend whose
/// compare-and-swap holds across processes
/// ([`crate::storage::driver_has_cross_process_cas`]). Only the driver name
/// is reported, never the URL (it may carry a password).
pub(crate) fn cluster_problem(inputs: &BootInputs<'_>) -> Option<String> {
    if !inputs.config.is_clustered() {
        return None;
    }
    let env_url = inputs
        .env
        .iter()
        .find(|(var, _)| var == crate::storage::STORAGE_URL_VAR)
        .map(|(_, value)| value.as_str());
    let Some(url) = inputs.config.storage_url_with(env_url) else {
        return Some(
            "advertise_url is set but no storage_url: nodes of a cluster need one shared backend"
                .to_string(),
        );
    };
    match crate::storage::StorageUrl::parse(&url) {
        Ok(parsed) if crate::storage::driver_has_cross_process_cas(parsed.driver()) => None,
        Ok(parsed) => Some(format!(
            "storage driver `{}` cannot exclude other nodes from a profile; use sqlite or mongodb",
            parsed.driver()
        )),
        Err(_) => Some("the storage URL does not parse".to_string()),
    }
}

/// Why the allowlisted tool groups cannot run safely, if they cannot.
fn sandbox_problems(inputs: &BootInputs<'_>) -> Vec<String> {
    if !needs_sandbox(inputs.config) {
        return Vec::new();
    }
    let sandbox = &inputs.config.sandbox;
    let mut problems = Vec::new();
    if !inputs.sandbox_available {
        problems.push("host_shell is allowlisted but Docker is not available".to_string());
    }
    if crate::profiles::tools::is_host_network(&sandbox.network) {
        problems.push("network `host` defeats the sandbox".to_string());
    }
    if sandbox.image.trim().is_empty() {
        problems.push("no image is set".to_string());
    }
    if sandbox.memory_limit_mb == 0 || !sandbox.cpu_limit.is_finite() || sandbox.cpu_limit <= 0.0 {
        problems.push("memory_limit_mb and cpu_limit must be positive and finite".to_string());
    }
    problems
}

/// Whether `config` allowlists a group that needs the container sandbox.
pub fn needs_sandbox(config: &SaasConfig) -> bool {
    crate::profiles::tools::parse_allowlist(&config.tool_allowlist)
        .iter()
        .any(|group| group.needs_sandbox())
}

fn root_problem(root: &Path, home: Option<&Path>) -> Option<String> {
    if !root.is_absolute() {
        return Some(format!("{} is not an absolute path", root.display()));
    }
    let meta = match std::fs::metadata(root) {
        Ok(meta) if meta.is_dir() => meta,
        Ok(_) => return Some(format!("{} is not a directory", root.display())),
        Err(e) => return Some(format!("{}: {e}", root.display())),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o002 != 0 {
            return Some(format!("{} is world-writable", root.display()));
        }
    }
    #[cfg(not(unix))]
    let _ = meta;
    if let Some(home) = home {
        // Compare resolved paths, so neither a symlink nor `..` can alias the
        // desktop directory.
        let resolve = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        let desktop_root = resolve(&home.join(".openhuman"));
        if resolve(root).starts_with(&desktop_root) {
            return Some(format!(
                "{} is inside the desktop app's {}",
                root.display(),
                desktop_root.display()
            ));
        }
    }
    None
}

fn service_flags(s: &ServiceSet) -> [(&'static str, bool); 11] {
    [
        ("rpc_http", s.rpc_http),
        ("socketio", s.socketio),
        ("cron", s.cron),
        ("channels", s.channels),
        ("login_gated", s.login_gated),
        ("update_scheduler", s.update_scheduler),
        ("memory_queue", s.memory_queue),
        ("skill_catalog_refresh", s.skill_catalog_refresh),
        ("mcp_boot", s.mcp_boot),
        ("integrations", s.integrations),
        ("memory_sync", s.memory_sync),
    ]
}

fn domain_flags(d: &DomainSet) -> [(&'static str, bool); 21] {
    [
        ("agent", d.agent),
        ("memory", d.memory),
        ("threads", d.threads),
        ("config", d.config),
        ("security", d.security),
        ("flows", d.flows),
        ("skills", d.skills),
        ("mcp", d.mcp),
        ("channels", d.channels),
        ("web3", d.web3),
        ("voice", d.voice),
        ("media", d.media),
        ("inference", d.inference),
        ("integrations", d.integrations),
        ("automation", d.automation),
        ("runtimes", d.runtimes),
        ("desktop", d.desktop),
        ("hosted", d.hosted),
        ("modules", d.modules),
        ("platform", d.platform),
        ("operator", d.operator),
    ]
}

#[cfg(test)]
#[path = "boot_guard_tests.rs"]
mod tests;

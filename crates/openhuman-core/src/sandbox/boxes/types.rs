//! The box registry's vocabulary: what a box is created with, how long it is
//! kept for its owner, the command it runs, what comes back, how it fails,
//! and the [`BoxDriver`] seam that creates, runs, closes and forgets boxes.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use sha2::{Digest, Sha256};
pub use tinybox_bus::Retention;

use crate::config::{Agent37RuntimeConfig, OpenShellRuntimeConfig};

/// The work folder of an OpenShell box: the base image's `/sandbox` workspace.
pub const OPENSHELL_WORKDIR: &str = "/sandbox";

/// The work folder of an agent37 box: its user's home.
pub const AGENT37_WORKDIR: &str = "/home/node";

/// The fewest seconds agent37 lets a box sit idle before it sleeps.
pub const MIN_IDLE_SLEEP_SECS: u32 = 300;

/// The most seconds agent37 lets a box sit idle before it sleeps.
pub const MAX_IDLE_SLEEP_SECS: u32 = 86_400;

/// Domain separation for [`OwnerScope`]'s hash, versioned like the owner key.
const OWNER_SCOPE_DOMAIN: &[u8] = b"openhuman.sandbox.box-scope.v1\0";

/// Where a tenant's box belongs beyond the tenant itself: the deployment's
/// namespace and the active workspace, hashed together.
///
/// A [`Tenant`](crate::core::runtime::Tenant) names who a box is for within
/// one host. Two hosts sharing a box service (or one host switching between
/// signed-in users) can see the same tenant, so the registry keeps a box per
/// scope and tenant, and a retained box's owner key folds the scope in. Only
/// the hash is kept: no namespace or path is ever sent anywhere. The default
/// scope is the unscoped one, for tests and drivers with no host around them.
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct OwnerScope {
    digest: Option<[u8; 32]>,
}

impl OwnerScope {
    /// The scope of boxes for `workspace` within the deployment `namespace`
    /// (blank for none). Each part is length-prefixed, so no two different
    /// pairs share a scope.
    pub fn new(namespace: &str, workspace: &Path) -> Self {
        let workspace = workspace.as_os_str().as_encoded_bytes();
        let mut hash = Sha256::new();
        hash.update(OWNER_SCOPE_DOMAIN);
        for part in [namespace.as_bytes(), workspace] {
            hash.update((part.len() as u64).to_le_bytes());
            hash.update(part);
        }
        Self {
            digest: Some(hash.finalize().into()),
        }
    }

    /// The scope's hash, or `None` for the unscoped default.
    pub fn digest(&self) -> Option<&[u8; 32]> {
        self.digest.as_ref()
    }
}

impl std::fmt::Debug for OwnerScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.digest {
            None => f.write_str("OwnerScope(unscoped)"),
            Some(digest) => write!(f, "OwnerScope({})", hex::encode(&digest[..6])),
        }
    }
}

/// What a new box is created with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxSettings {
    /// OCI image reference the box starts from.
    pub image: String,
    /// CPU allowance in thousandths of a core.
    pub cpu_millis: u32,
    /// Memory limit in bytes, always a whole number of mebibytes.
    pub memory_bytes: u64,
    /// The folder commands run in and file paths are relative to.
    pub workdir: String,
    /// The scope the calling tenant's box belongs to; the registry keeps one
    /// box per scope and tenant.
    pub owner_scope: OwnerScope,
}

impl BoxSettings {
    /// The settings `[runtime.openshell]` gives every new box.
    ///
    /// # Errors
    ///
    /// [`BoxError::InvalidConfig`] when `image` is blank or `cpu_millis` or
    /// `memory_mb` is zero; [`BoxError::InvalidMemory`] when `memory_mb` does
    /// not fit in bytes.
    pub fn from_config(config: &OpenShellRuntimeConfig) -> Result<Self, BoxError> {
        if config.image.trim().is_empty() {
            return Err(BoxError::InvalidConfig(
                "[runtime.openshell] image must not be blank",
            ));
        }
        if config.cpu_millis == 0 || config.memory_mb == 0 {
            return Err(BoxError::InvalidConfig(
                "[runtime.openshell] cpu_millis and memory_mb must be positive",
            ));
        }
        let memory_bytes = config.memory_bytes().ok_or(BoxError::InvalidMemory {
            memory_mb: config.memory_mb,
        })?;
        Ok(Self {
            image: config.image.clone(),
            cpu_millis: config.cpu_millis,
            memory_bytes,
            workdir: OPENSHELL_WORKDIR.to_owned(),
            owner_scope: OwnerScope::default(),
        })
    }

    /// The settings `[runtime.agent37]` gives every new box. `image` carries
    /// the template name (agent37 starts a box from a registered template,
    /// not an OCI image). The service snaps CPU and memory up to a fixed
    /// instance shape, so they are a request: a request over the largest
    /// shape is logged, and a memory size too large to express in bytes asks
    /// for the service's maximum.
    ///
    /// # Errors
    ///
    /// [`BoxError::InvalidConfig`] when `template` is blank.
    pub fn from_agent37(config: &Agent37RuntimeConfig) -> Result<Self, BoxError> {
        if config.template.trim().is_empty() {
            return Err(BoxError::InvalidConfig(
                "[runtime.agent37] template must not be blank",
            ));
        }
        if config.exceeds_largest_shape() {
            tracing::warn!(
                cpu_millis = config.cpu_millis,
                memory_mb = config.memory_mb,
                "[sandbox:box] agent37 request exceeds the largest instance shape and will be capped"
            );
        }
        Ok(Self {
            image: config.template.clone(),
            cpu_millis: config.cpu_millis,
            memory_bytes: config.memory_bytes().unwrap_or(u64::MAX),
            workdir: AGENT37_WORKDIR.to_owned(),
            owner_scope: OwnerScope::default(),
        })
    }
}

/// How long a retaining backend keeps a tenant's box: it sleeps after
/// `idle_sleep_secs` without a command and is deleted after
/// `delete_after_secs` asleep. The same for every tenant of a registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetainPolicy {
    /// Seconds without a command before the box falls asleep.
    pub idle_sleep_secs: u32,
    /// Seconds a box may sleep before the backend deletes it.
    pub delete_after_secs: u64,
}

impl RetainPolicy {
    /// The policy `[runtime.agent37]` sets: `idle_sleep_secs` clamped to the
    /// range agent37 accepts (300 to 86400), and a zero `delete_after_secs`
    /// (which would delete a box as soon as it sleeps) raised to 1.
    pub fn from_agent37(config: &Agent37RuntimeConfig) -> Self {
        Self {
            idle_sleep_secs: config
                .idle_sleep_secs
                .clamp(MIN_IDLE_SLEEP_SECS, MAX_IDLE_SLEEP_SECS),
            delete_after_secs: config.delete_after_secs.max(1),
        }
    }

    /// This policy for the box of the owner `key` names, as the box contract's
    /// [`Retention`].
    pub fn for_owner(self, key: String) -> Retention {
        Retention {
            key,
            idle_sleep_secs: self.idle_sleep_secs,
            delete_after_secs: self.delete_after_secs,
        }
    }
}

/// Registry-wide bounds, fixed when the registry is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryLimits {
    /// How long a box may sit idle before it is closed.
    pub idle_ttl: Duration,
    /// The most boxes alive at once.
    pub max_live: usize,
    /// `Some` for a backend that keeps each tenant's box for it: boxes are
    /// created with the tenant's [`Retention`], and the registry only
    /// ever forgets them ([`BoxDriver::forget`]), never closes them.
    pub retain: Option<RetainPolicy>,
}

impl RegistryLimits {
    /// The bounds `[runtime.openshell]` sets. A zero `max_live` would refuse
    /// every command and a zero `idle_ttl_secs` would reap every box at once,
    /// so each is raised to 1.
    pub fn from_config(config: &OpenShellRuntimeConfig) -> Self {
        Self {
            idle_ttl: Duration::from_secs(config.idle_ttl_secs.max(1)),
            max_live: config.max_live.max(1),
            retain: None,
        }
    }

    /// The bounds `[runtime.agent37]` sets. A box is kept for its tenant
    /// ([`RetainPolicy::from_agent37`]); the registry lets go of its handle
    /// once the box has been idle as long as it takes to fall asleep, and a
    /// zero `max_live` is raised to 1.
    pub fn from_agent37(config: &Agent37RuntimeConfig) -> Self {
        let retain = RetainPolicy::from_agent37(config);
        Self {
            idle_ttl: Duration::from_secs(u64::from(retain.idle_sleep_secs)),
            max_live: config.max_live.max(1),
            retain: Some(retain),
        }
    }
}

/// One shell command for a tenant's box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxRequest {
    /// The command line, run with `sh -c`.
    pub command: String,
    /// The only variables set for the command; nothing from the host.
    pub env: BTreeMap<String, String>,
    /// How long the command may run.
    pub timeout: Duration,
}

/// What a driver runs inside a box: the program, already wrapped in the
/// box-side deadline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxExec {
    /// Unshelled program and arguments.
    pub argv: Vec<String>,
    /// Working directory inside the box.
    pub cwd: String,
    /// Variables set for the program.
    pub env: BTreeMap<String, String>,
    /// When the registry gives up on the reply; a driver's own transport
    /// deadline must not fire before it.
    pub deadline: Duration,
}

/// What a command left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxOutput {
    /// The program's exit status.
    pub exit_code: i32,
    /// Standard output, unmodified.
    pub stdout: Vec<u8>,
    /// Standard error, unmodified.
    pub stderr: Vec<u8>,
}

/// Why a command could not run in a box. The messages are written to read
/// well as a tool result.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BoxError {
    /// Every box is in use and none has been idle long enough to reap.
    #[error("the sandbox service is at capacity; try again shortly")]
    AtCapacity,
    /// The box is gone: closed, lost, or no longer able to run commands.
    #[error("the sandbox box is no longer available")]
    UnknownResource,
    /// The box was released or shut down while the command was waiting on it.
    #[error("the sandbox was released while the command was running")]
    Released,
    /// The call carries no tenant, so it has no box of its own.
    #[error("no tenant scope; refusing to use a shared sandbox")]
    NoTenant,
    /// The configured sandbox backend keeps no box per tenant, so there is no
    /// box to run in or move files to.
    #[error("the configured sandbox backend keeps no per-tenant box")]
    NoBoxBackend,
    /// The box runs on a hosted service, which local-only privacy forbids.
    #[error(
        "the sandbox runs on a hosted service, which [privacy] mode = \"local_only\" does not permit"
    )]
    LocalOnly,
    /// `[runtime.openshell] memory_mb` is too large to express in bytes.
    #[error("sandbox memory of {memory_mb} MiB is too large")]
    InvalidMemory {
        /// The configured value.
        memory_mb: u64,
    },
    /// A `[runtime.openshell]` or `[runtime.agent37]` value is out of range;
    /// the message names the table.
    #[error("invalid sandbox setting: {0}")]
    InvalidConfig(&'static str),
    /// A variable for the command is not UTF-8.
    #[error("sandbox environment is not UTF-8")]
    NonUtf8Env,
    /// A file path for the box is not a file inside its work folder.
    #[error("the sandbox file path is not allowed: {0}")]
    InvalidPath(&'static str),
    /// A file is over the transfer limit.
    #[error("the file is {size} bytes, over the {limit} byte limit")]
    FileTooLarge {
        /// The file's size in bytes.
        size: u64,
        /// The limit it is over.
        limit: u64,
    },
    /// The box has no regular file at the path.
    #[error("there is no file `{0}` in the sandbox")]
    FileNotFound(String),
    /// Fewer bytes of a file arrived than it holds.
    #[error("the sandbox returned {got} of the file's {expected} bytes")]
    ShortRead {
        /// The file's size in bytes.
        expected: u64,
        /// The bytes that arrived.
        got: u64,
    },
    /// A command moving a file failed in the box.
    #[error("could not {action} `{path}` in the sandbox: {detail}")]
    FileTransfer {
        /// `read` or `write`.
        action: &'static str,
        /// The path, relative to the work folder.
        path: String,
        /// What the box said, or why the command failed.
        detail: String,
    },
    /// The driver failed for another reason.
    #[error("the sandbox service failed: {0}")]
    Driver(String),
}

/// Creates, runs, closes and forgets boxes. The registry owns their
/// lifecycle; a driver only performs each step.
#[async_trait]
pub trait BoxDriver: Send + Sync + 'static {
    /// Create a box and return its resource id. With `retention`, the box is
    /// kept for its owner: a backend that already keeps one under the same
    /// key hands that one back, and a backend that cannot retain boxes
    /// refuses rather than create a disposable one.
    async fn create(
        &self,
        settings: &BoxSettings,
        retention: Option<&Retention>,
    ) -> Result<String, BoxError>;

    /// Run one program in the box and collect its output. A box that is gone
    /// fails with [`BoxError::UnknownResource`].
    async fn exec(&self, resource: &str, exec: &BoxExec) -> Result<BoxOutput, BoxError>;

    /// Close the box and release what it holds.
    async fn close(&self, resource: &str) -> Result<(), BoxError>;

    /// Let go of a retained box: drop this host's handle on it and leave the
    /// box, and its files, to its backend, which sleeps it when idle and
    /// deletes it only once it has slept past its retention.
    async fn forget(&self, resource: &str) -> Result<(), BoxError>;
}

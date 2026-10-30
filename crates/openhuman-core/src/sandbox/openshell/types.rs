//! The box registry's vocabulary: what a box is created with, the command it
//! runs, what comes back, how it fails, and the [`BoxDriver`] seam that
//! creates, runs and closes boxes.

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;

use crate::config::OpenShellRuntimeConfig;

/// What a new box is created with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxSettings {
    /// OCI image reference the box starts from.
    pub image: String,
    /// CPU allowance in thousandths of a core.
    pub cpu_millis: u32,
    /// Memory limit in bytes, always a whole number of mebibytes.
    pub memory_bytes: u64,
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
            return Err(BoxError::InvalidConfig("image must not be blank"));
        }
        if config.cpu_millis == 0 || config.memory_mb == 0 {
            return Err(BoxError::InvalidConfig(
                "cpu_millis and memory_mb must be positive",
            ));
        }
        let memory_bytes = config.memory_bytes().ok_or(BoxError::InvalidMemory {
            memory_mb: config.memory_mb,
        })?;
        Ok(Self {
            image: config.image.clone(),
            cpu_millis: config.cpu_millis,
            memory_bytes,
        })
    }
}

/// Registry-wide bounds, fixed when the registry is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryLimits {
    /// How long a box may sit idle before it is closed.
    pub idle_ttl: Duration,
    /// The most boxes alive at once.
    pub max_live: usize,
}

impl RegistryLimits {
    /// The bounds `[runtime.openshell]` sets. A zero `max_live` would refuse
    /// every command and a zero `idle_ttl_secs` would reap every box at once,
    /// so each is raised to 1.
    pub fn from_config(config: &OpenShellRuntimeConfig) -> Self {
        Self {
            idle_ttl: Duration::from_secs(config.idle_ttl_secs.max(1)),
            max_live: config.max_live.max(1),
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
    #[error("no tenant scope; refusing to run a command in a shared sandbox")]
    NoTenant,
    /// `[runtime.openshell] memory_mb` is too large to express in bytes.
    #[error("sandbox memory of {memory_mb} MiB is too large")]
    InvalidMemory {
        /// The configured value.
        memory_mb: u64,
    },
    /// A `[runtime.openshell]` value is out of range.
    #[error("invalid [runtime.openshell] setting: {0}")]
    InvalidConfig(&'static str),
    /// A variable for the command is not UTF-8.
    #[error("sandbox environment is not UTF-8")]
    NonUtf8Env,
    /// The driver failed for another reason.
    #[error("the sandbox service failed: {0}")]
    Driver(String),
}

/// Creates, runs and closes boxes. The registry owns their lifecycle; a
/// driver only performs each step.
#[async_trait]
pub trait BoxDriver: Send + Sync + 'static {
    /// Create a box and return its resource id.
    async fn create(&self, settings: &BoxSettings) -> Result<String, BoxError>;

    /// Run one program in the box and collect its output. A box that is gone
    /// fails with [`BoxError::UnknownResource`].
    async fn exec(&self, resource: &str, exec: &BoxExec) -> Result<BoxOutput, BoxError>;

    /// Close the box and release what it holds.
    async fn close(&self, resource: &str) -> Result<(), BoxError>;
}

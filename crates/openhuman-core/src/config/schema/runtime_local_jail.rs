//! `[runtime.local_jail]` — filesystem grants for the local OS jail
//! (Landlock on Linux, Seatbelt on macOS).
//!
//! With a real jail in force, a command can touch only what the policy grants.
//! The built-in set covers the everyday toolchain (`~/.cargo`, `~/.rustup`,
//! `~/.nvm`, `~/.npm`, `/usr/local`, `/opt`, git config); this section extends
//! or trims it.
//!
//! Credential stores (`~/.ssh`, `~/.gnupg`, `~/.aws`, ...) are never granted:
//! entries naming one, or a parent that would expose one (such as `~`), are
//! dropped with a warning. Paths accept a leading `~/`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct LocalJailConfig {
    /// Grant the built-in toolchain homes that exist on this machine
    /// (`~/.cargo` read-write; `~/.rustup`, `~/.nvm`, `~/.npm`, `/usr/local`
    /// and `/opt` read-only) plus the user's git config. Set `false` to
    /// confine commands to the workspace, the system baseline and
    /// `extra_read_only` / `extra_read_write`.
    #[serde(default = "default_true")]
    pub toolchain_homes: bool,
    /// Extra read-only (and executable) paths. Missing paths are skipped.
    #[serde(default)]
    pub extra_read_only: Vec<String>,
    /// Extra read-write paths. Missing paths are skipped.
    #[serde(default)]
    pub extra_read_write: Vec<String>,
    /// Let jailed commands read `/proc`. **Off by default**: `/proc/<pid>/environ`
    /// and `/proc/<pid>/cmdline` expose the environment and arguments of every
    /// process the user owns, including the core's own credentials. Enable it
    /// only when a workflow needs tools such as `ps` or `top`.
    #[serde(default)]
    pub allow_proc: bool,
}

fn default_true() -> bool {
    true
}

impl Default for LocalJailConfig {
    fn default() -> Self {
        Self {
            toolchain_homes: true,
            extra_read_only: Vec::new(),
            extra_read_write: Vec::new(),
            allow_proc: false,
        }
    }
}

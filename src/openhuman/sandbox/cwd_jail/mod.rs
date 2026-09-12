//! Directory jail (cwd_jail): jail an agent/tool into a single workspace.
//!
//! ## Why this exists
//!
//! `src/openhuman/security/` already has a `Sandbox` trait that wraps
//! `Command`s (Landlock / Firejail / Bubblewrap / Docker). It works well
//! for Linux but the macOS branch is a stub (`bwrap` doesn't exist there)
//! and there is no Windows backend at all. Callers also have to thread
//! `SecurityConfig` through every call site.
//!
//! `cwd_jail` is the user-facing facade. Callers describe *what* the
//! jail looks like ([`Jail`]) and the module picks the right OS backend:
//!
//! | OS      | Backend       | Mechanism                                  |
//! |---------|---------------|--------------------------------------------|
//! | Linux   | landlock      | Kernel 5.13+ LSM, applied in `pre_exec`    |
//! | macOS   | seatbelt      | `sandbox-exec -p '<profile>' …`            |
//! | Windows | appcontainer  | `CreateAppContainerProfile` + `STARTUPINFOEX` |
//! | other   | noop          | Plain `Command::spawn`, audit-only         |
//!
//! ## Quick start
//!
//! ```ignore
//! use openhuman::openhuman::sandbox::cwd_jail::{spawn, Jail};
//! use std::process::Command;
//!
//! let mut jail = Jail::new("/Users/x/work/proj", "agent.delegate")
//!     .add_read_only("/usr/lib")
//!     .deny_subprocess();
//! jail.canonicalize_or_log();
//!
//! let mut cmd = Command::new("node");
//! cmd.arg("script.js");
//! let child = spawn(&jail, cmd)?;
//! ```
//!
//! ## What this does *not* do
//!
//! - It does not jail the current process. Backends spawn a child. The core
//!   itself is trusted; only the things it shells out to are caged.
//! - It does not replace `security::SecurityPolicy`. The autonomy gate
//!   still decides *whether* a command may run; this module decides
//!   *what filesystem* it sees once approved.
//! - It does not encrypt files. ACLs / Landlock rules / Seatbelt profiles
//!   are the wall — anything inside `root` is fully visible to the child.

pub mod detect;
pub mod jail;
pub mod noop;
pub mod registry;

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

pub use jail::{Jail, JailBackend};
pub use noop::{NoopBackend, NOOP_BACKEND_NAME};
pub use registry::{JailRecord, JailRegistry};

use std::process::{Child, Command};
use std::sync::{Arc, OnceLock};

/// Cached default backend for the current platform.
static DEFAULT_BACKEND: OnceLock<Arc<dyn JailBackend>> = OnceLock::new();

/// Returns the process-wide default backend, lazily auto-detected.
pub fn default_backend() -> Arc<dyn JailBackend> {
    DEFAULT_BACKEND.get_or_init(detect::pick_backend).clone()
}

/// Spawn `cmd` inside the jail described by `spawn`, using the default backend.
///
/// `jail.canonicalize()` is called once here so the backends never see
/// `..` or symlinks. If the root does not exist, the spawn fails with
/// `NotFound` (canonicalize bubbles it up) — callers should create the
/// workspace before encapsulating.
pub fn spawn(jail: &Jail, cmd: Command) -> std::io::Result<Child> {
    let mut jail = jail.clone();
    jail.canonicalize()?;
    default_backend().spawn(&jail, cmd)
}

/// Same as [`jail`] but with a caller-supplied backend. Useful in
/// tests and for callers that want to opt into a weaker backend
/// explicitly (e.g. forcing [`NoopBackend`] during local dev).
pub fn spawn_with(backend: &dyn JailBackend, jail: &Jail, cmd: Command) -> std::io::Result<Child> {
    let mut jail = jail.clone();
    jail.canonicalize()?;
    backend.spawn(&jail, cmd)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

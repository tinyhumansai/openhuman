//! macOS backend: Seatbelt via `sandbox-exec`.
//!
//! `sandbox-exec` is a built-in macOS binary that takes a Scheme-style
//! profile (the "Seatbelt" / TrustedBSD policy language) and execs the
//! requested command under it. Chromium, iOS simulators and Apple's own
//! tools use the same SPI under the hood. The CLI is technically
//! deprecated but has stayed shipping for a decade and is the only
//! supported way to apply Seatbelt without private framework bindings.

#![cfg(target_os = "macos")]

use std::process::{Child, Command};

use super::jail::{Jail, JailBackend};

pub struct SeatbeltBackend;

impl Default for SeatbeltBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SeatbeltBackend {
    pub fn new() -> Self {
        Self
    }
}

impl JailBackend for SeatbeltBackend {
    fn name(&self) -> &'static str {
        "seatbelt"
    }

    fn is_available(&self) -> bool {
        std::path::Path::new("/usr/bin/sandbox-exec").exists()
    }

    fn spawn(&self, jail: &Jail, cmd: Command) -> std::io::Result<Child> {
        let profile = render_profile(jail);

        // sandbox-exec only accepts profiles from disk or from `-p`. Inline
        // (`-p`) is simpler and avoids a tempfile lifecycle problem (the
        // child may outlive our parent scope).
        let program = cmd.get_program().to_os_string();
        let args: Vec<_> = cmd.get_args().map(|a| a.to_os_string()).collect();
        let envs: Vec<_> = cmd
            .get_envs()
            .map(|(k, v)| (k.to_os_string(), v.map(|s| s.to_os_string())))
            .collect();
        let cwd = cmd.get_current_dir().map(|p| p.to_path_buf());

        let mut wrapper = Command::new("/usr/bin/sandbox-exec");
        wrapper.arg("-p").arg(profile).arg(program).args(args);
        for (k, v) in envs {
            match v {
                Some(val) => {
                    wrapper.env(k, val);
                }
                None => {
                    wrapper.env_remove(k);
                }
            }
        }
        if let Some(d) = cwd {
            wrapper.current_dir(d);
        }
        // Inherit stdio from the original command intent. `std::process`
        // doesn't expose the original `Stdio`, so we leave the inherited
        // defaults — callers can re-wire by spawning into a pre-set stdio
        // via the returned `Child` is not possible; for now we match the
        // sandbox-exec defaults (inherit). Document this in mod.rs.
        wrapper.spawn()
    }
}

/// Render a Seatbelt profile.
///
/// The model is **allow-default for reads, deny-default for writes**.
/// A deny-everything profile is unworkable on macOS — Mach-O binaries need
/// dyld, libsystem, the shared cache, mach lookups for Foundation, and a
/// dozen other things that change between OS releases. Locking those down
/// breaks tools faster than it stops attackers.
///
/// What we actually want from a *directory jail* is: the child can read
/// pretty much anything, but it can only **write** inside `jail.root` and
/// the system scratchpad. That's what this profile enforces.
fn render_profile(jail: &Jail) -> String {
    let mut out = String::new();
    out.push_str("(version 1)\n");
    out.push_str("(allow default)\n");

    // Network gate. `allow default` enables network*; only flip it off
    // when the jail explicitly denies it.
    if !jail.allow_net {
        out.push_str("(deny network*)\n");
    }

    // Subprocess gate. Same idea — only restrict on opt-in.
    if !jail.allow_subprocess {
        out.push_str("(deny process-fork)\n");
        out.push_str("(deny process-exec)\n");
    }

    // The actual directory jail: deny writes everywhere, then re-allow
    // them under root + /private/tmp (the macOS scratchpad most tools
    // assume exists and is writable).
    out.push_str("(deny file-write*)\n");
    out.push_str(&format!(
        "(allow file-write*\n  (subpath \"{}\")\n  (subpath \"/private/tmp\")\n)\n",
        escape(&jail.root.to_string_lossy())
    ));

    // `read_only` is informational on macOS — reads are already allowed
    // by `(allow default)`. We keep the field on `Jail` because Landlock
    // and AppContainer use it, and it lets callers express intent
    // uniformly across platforms.
    let _ = jail.read_only.len();

    out
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
#[path = "macos_tests.rs"]
mod tests;

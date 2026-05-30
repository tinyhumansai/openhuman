//! Cross-platform directory-jail facade.
//!
//! A [`Jail`] describes *what* the agent is allowed to see; a [`JailBackend`]
//! enforces it on a specific OS. Callers only interact with [`Jail`] and the
//! top-level [`crate::openhuman::cwd_jail::spawn`] function — they
//! never pick a backend by name.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};

/// Declarative description of a directory jail.
///
/// One `root` (read/write), zero or more `read_only` paths, an optional
/// allow-list of extra paths the child *may* read, and a network toggle.
/// Backends translate this into Landlock rules, a Seatbelt profile, or an
/// AppContainer ACL.
#[derive(Debug, Clone)]
pub struct Jail {
    /// Primary read/write root. The child cannot escape this directory for
    /// writes. Must be an existing, canonicalizable directory.
    pub root: PathBuf,
    /// Extra paths the child may read (e.g. `/usr/lib`, the runtime-node
    /// install). Writes are still denied.
    pub read_only: Vec<PathBuf>,
    /// Allow outbound network. Most agent tools need this; some risky tools
    /// (untrusted code execution) should disable it.
    pub allow_net: bool,
    /// Allow the child to spawn further child processes. AppContainer and
    /// Seatbelt can deny this; Landlock cannot.
    pub allow_subprocess: bool,
    /// Free-form label used by audit logs and (on Windows) as the basis for
    /// the AppContainer profile name. Keep it short and ASCII.
    pub label: String,
}

impl Jail {
    /// Convenience: read/write jail rooted at `root` with networking enabled
    /// and no additional read-only mounts.
    pub fn new(root: impl AsRef<Path>, label: impl Into<String>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            read_only: Vec::new(),
            allow_net: true,
            allow_subprocess: true,
            label: label.into(),
        }
    }

    pub fn add_read_only(mut self, path: impl AsRef<Path>) -> Self {
        self.read_only.push(path.as_ref().to_path_buf());
        self
    }

    pub fn deny_net(mut self) -> Self {
        self.allow_net = false;
        self
    }

    pub fn deny_subprocess(mut self) -> Self {
        self.allow_subprocess = false;
        self
    }

    /// Canonicalize `root` and `read_only` so backends never see `..` or
    /// symlink trickery. Returns an error if `root` does not exist.
    pub fn canonicalize(&mut self) -> std::io::Result<()> {
        self.root = self.root.canonicalize()?;
        for p in self.read_only.iter_mut() {
            if let Ok(c) = p.canonicalize() {
                *p = c;
            }
        }
        Ok(())
    }

    /// Best-effort canonicalize that swallows errors and logs them. Most
    /// callers should use the validating [`Jail::canonicalize`] path that
    /// [`crate::openhuman::cwd_jail::spawn`] runs automatically.
    pub fn canonicalize_or_log(&mut self) {
        if let Err(e) = self.canonicalize() {
            log::warn!(
                "[cwd_jail] failed to canonicalize jail root {}: {}",
                self.root.display(),
                e
            );
        }
    }
}

/// OS-specific enforcement of a [`Jail`].
///
/// We model spawning rather than `Command` mutation because Windows
/// AppContainer requires custom `CreateProcess` flags that `std`'s
/// `Command::spawn` does not expose.
pub trait JailBackend: Send + Sync {
    /// Stable identifier, used in logs / audit ("landlock", "seatbelt",
    /// "appcontainer", "noop").
    fn name(&self) -> &'static str;

    /// Whether the backend can actually enforce the jail in this process /
    /// on this kernel build. Auto-detection consults this before returning
    /// a backend.
    fn is_available(&self) -> bool;

    /// Spawn `cmd` under the jail described by `jail`. Backends own how the
    /// jail is materialized (Landlock ruleset, sandbox-exec wrapper,
    /// AppContainer profile + restricted token).
    fn spawn(&self, jail: &Jail, cmd: Command) -> std::io::Result<Child>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_permissive() {
        let j = Jail::new("/tmp", "x");
        assert!(j.allow_net);
        assert!(j.allow_subprocess);
        assert_eq!(j.label, "x");
        assert!(j.read_only.is_empty());
    }

    #[test]
    fn deny_net_is_idempotent() {
        let j = Jail::new("/tmp", "x").deny_net().deny_net();
        assert!(!j.allow_net);
    }

    #[test]
    fn deny_subprocess_is_idempotent() {
        let j = Jail::new("/tmp", "x").deny_subprocess().deny_subprocess();
        assert!(!j.allow_subprocess);
    }

    #[test]
    fn add_read_only_appends_in_order() {
        let j = Jail::new("/tmp", "x")
            .add_read_only("/a")
            .add_read_only("/b")
            .add_read_only("/c");
        assert_eq!(j.read_only.len(), 3);
        assert_eq!(j.read_only[0], PathBuf::from("/a"));
        assert_eq!(j.read_only[2], PathBuf::from("/c"));
    }

    #[test]
    fn canonicalize_resolves_real_path() {
        let dir = std::env::temp_dir();
        let mut j = Jail::new(&dir, "x");
        j.canonicalize().unwrap();
        // After canonicalize, root has no `..` and resolves to a real path.
        assert!(j.root.is_absolute());
        assert!(j.root.exists());
    }

    #[test]
    fn canonicalize_swallows_missing_read_only() {
        // read_only entries that don't exist are silently dropped from
        // canonicalization (they stay as-is). Verify no panic.
        let dir = std::env::temp_dir();
        let mut j = Jail::new(&dir, "x").add_read_only("/this/never/existed");
        j.canonicalize().unwrap();
        assert_eq!(j.read_only.len(), 1);
    }

    #[test]
    fn canonicalize_errors_on_missing_root() {
        let mut j = Jail::new("/no/such/root/here", "x");
        let err = j.canonicalize().unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }
}

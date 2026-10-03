//! The filesystem grant set for the local OS jail.
//!
//! A real jail (Landlock, Seatbelt) denies everything it is not told about, so
//! the host must grant what everyday commands need: the toolchain homes
//! (`cargo`, `rustc`, `node`, `npm`) and the user's git config. The set is
//! computed from [`LocalJailConfig`] and the machine, once per policy; per-call
//! directories (output capture, `TMPDIR` scratch) are added by `ops`.
//!
//! **Credential floor.** `~/.ssh`, `~/.gnupg`, `~/.aws` and the rest of
//! `SecurityPolicy::is_always_forbidden` are never granted, whatever the config
//! says. Landlock grants are recursive, so a grant on a *parent* of a credential
//! store exposes it too; those are dropped as well. `/proc` is reachable only
//! through `LocalJailConfig::allow_proc`, never through the `extra_*` lists.

use crate::config::LocalJailConfig;
use crate::security::SecurityPolicy;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Credential directories under `$HOME` that no grant may reach or contain.
const HOME_CREDENTIAL_DIRS: &[&str] = &[".ssh", ".gnupg", ".aws", ".azure", ".kube"];

/// System toolchain roots granted read-only when present.
const SYSTEM_TOOLCHAIN_DIRS: &[&str] = &["/usr/local", "/opt"];

/// `$HOME`-relative toolchain homes granted read-only when present.
const HOME_READ_ONLY_DIRS: &[&str] = &[".rustup", ".nvm", ".npm"];

/// Files Cargo needs read access to inside `~/.cargo` when the directory also
/// holds registry credentials and so cannot be granted whole.
const CARGO_READ_ONLY_FILES: &[&str] = &["config.toml", "config", "env"];

/// Git config files read at startup, relative to `$HOME`.
const GITCONFIG_FILES: &[&str] = &[".gitconfig", ".config/git/config", ".config/git/ignore"];

/// Deepest `include` chain followed when collecting git config files.
const GITCONFIG_MAX_DEPTH: usize = 8;

/// Paths the jail is granted beyond the workspace root, all canonical and
/// existing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JailGrants {
    pub read_only: Vec<PathBuf>,
    pub read_write: Vec<PathBuf>,
}

/// Build the grant set for `home` (the user's home directory, if known).
pub fn resolve_local_jail_grants(home: Option<&Path>, cfg: &LocalJailConfig) -> JailGrants {
    let home = home.map(|h| h.canonicalize().unwrap_or_else(|_| h.to_path_buf()));
    let mut b = Builder::new(home.as_deref());

    if cfg.toolchain_homes {
        if let Some(h) = home.as_deref() {
            b.add_cargo_home(&h.join(".cargo"));
            for dir in HOME_READ_ONLY_DIRS {
                b.read_only(&h.join(dir), "toolchain");
            }
        }
        for dir in SYSTEM_TOOLCHAIN_DIRS {
            b.read_only(Path::new(dir), "toolchain");
        }
        if let Some(h) = home.as_deref() {
            b.add_gitconfig(h);
        }
    }
    for raw in &cfg.extra_read_only {
        if let Some(p) = b.expand(raw) {
            b.read_only(&p, "config");
        }
    }
    for raw in &cfg.extra_read_write {
        if let Some(p) = b.expand(raw) {
            b.read_write(&p, "config");
        }
    }
    // The one sanctioned route to `/proc`: it bypasses the floor deliberately.
    if cfg.allow_proc && Path::new("/proc").exists() {
        tracing::debug!("[sandbox:grants] granting read-only /proc (allow_proc = true)");
        b.grants.read_only.push(PathBuf::from("/proc"));
    }
    tracing::debug!(
        read_only = b.grants.read_only.len(),
        read_write = b.grants.read_write.len(),
        "[sandbox:grants] resolved local jail grants"
    );
    b.grants
}

struct Builder<'a> {
    home: Option<&'a Path>,
    grants: JailGrants,
    seen: HashSet<PathBuf>,
}

impl<'a> Builder<'a> {
    fn new(home: Option<&'a Path>) -> Self {
        Self {
            home,
            grants: JailGrants::default(),
            seen: HashSet::new(),
        }
    }

    /// Expand a leading `~` / `~/` against the home directory.
    fn expand(&self, raw: &str) -> Option<PathBuf> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        if raw == "~" {
            return self.home.map(Path::to_path_buf);
        }
        if let Some(rest) = raw.strip_prefix("~/") {
            return self.home.map(|h| h.join(rest));
        }
        Some(PathBuf::from(raw))
    }

    /// Canonicalize `path` and return it only if it exists and clears the
    /// credential floor. Canonicalizing first means a symlink into `~/.ssh`
    /// is judged by its target.
    fn admit(&self, path: &Path, source: &str) -> Option<PathBuf> {
        let canonical = match path.canonicalize() {
            Ok(c) => c,
            Err(_) => {
                tracing::debug!(source, path = %path.display(), "[sandbox:grants] skipped missing path");
                return None;
            }
        };
        if SecurityPolicy::is_always_forbidden(&canonical) || self.exposes_credentials(&canonical) {
            tracing::warn!(
                source,
                path = %canonical.display(),
                "[sandbox:grants] refused: grant would reach a credential store or system root"
            );
            return None;
        }
        Some(canonical)
    }

    /// True when `path` is, contains, or sits inside a credential directory.
    fn exposes_credentials(&self, path: &Path) -> bool {
        let Some(home) = self.home else { return false };
        HOME_CREDENTIAL_DIRS.iter().any(|d| {
            let cred = home.join(d);
            let cred = cred.canonicalize().unwrap_or(cred);
            path.starts_with(&cred) || cred.starts_with(path)
        })
    }

    fn read_only(&mut self, path: &Path, source: &str) {
        if let Some(p) = self.admit(path, source) {
            if self.seen.insert(p.clone()) {
                self.grants.read_only.push(p);
            }
        }
    }

    fn read_write(&mut self, path: &Path, source: &str) {
        if let Some(p) = self.admit(path, source) {
            // A path granted writable subsumes an earlier read-only grant.
            self.grants.read_only.retain(|g| g != &p);
            self.seen.insert(p.clone());
            if !self.grants.read_write.contains(&p) {
                self.grants.read_write.push(p);
            }
        }
    }

    /// `~/.cargo` is read-write (registry, git checkouts, `cargo install`).
    /// When it also holds registry credentials (`credentials.toml`), granting
    /// it whole would hand the token to every jailed command, so grant only the
    /// parts Cargo needs.
    fn add_cargo_home(&mut self, cargo: &Path) {
        if !cargo.is_dir() {
            return;
        }
        let has_credentials = ["credentials.toml", "credentials"]
            .iter()
            .any(|f| cargo.join(f).exists());
        if !has_credentials {
            self.read_write(cargo, "toolchain");
            return;
        }
        tracing::debug!("[sandbox:grants] ~/.cargo holds credentials; granting its parts only");
        self.read_only(&cargo.join("bin"), "toolchain");
        self.read_write(&cargo.join("registry"), "toolchain");
        self.read_write(&cargo.join("git"), "toolchain");
        for f in CARGO_READ_ONLY_FILES {
            self.read_only(&cargo.join(f), "toolchain");
        }
    }

    /// Read-only grants for the user's git config files and everything they
    /// `include`, each canonicalized (a config is often a symlink into a
    /// dotfiles repo, and the jail needs the target).
    fn add_gitconfig(&mut self, home: &Path) {
        let mut roots: Vec<PathBuf> = GITCONFIG_FILES.iter().map(|f| home.join(f)).collect();
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            roots.push(Path::new(&xdg).join("git/config"));
        }
        let mut visited = HashSet::new();
        for root in roots {
            self.add_gitconfig_file(&root, home, 0, &mut visited);
        }
    }

    fn add_gitconfig_file(
        &mut self,
        file: &Path,
        home: &Path,
        depth: usize,
        visited: &mut HashSet<PathBuf>,
    ) {
        let Ok(canonical) = file.canonicalize() else {
            return;
        };
        if !canonical.is_file() || !visited.insert(canonical.clone()) {
            return;
        }
        self.read_only(&canonical, "gitconfig");
        if depth >= GITCONFIG_MAX_DEPTH {
            return;
        }
        let Ok(text) = std::fs::read_to_string(&canonical) else {
            return;
        };
        // Relative includes resolve against the directory of the file *as
        // named*, which for a symlinked config is the link's directory.
        let base = file.parent().unwrap_or(home);
        for target in include_paths(&text) {
            let resolved = if let Some(rest) = target.strip_prefix("~/") {
                home.join(rest)
            } else if Path::new(&target).is_absolute() {
                PathBuf::from(&target)
            } else {
                base.join(&target)
            };
            self.add_gitconfig_file(&resolved, home, depth + 1, visited);
        }
    }
}

/// `path = …` values inside `[include]` / `[includeIf …]` sections.
fn include_paths(config: &str) -> Vec<String> {
    let mut in_include = false;
    let mut out = Vec::new();
    for line in config.lines() {
        let line = line.trim();
        if let Some(section) = line.strip_prefix('[') {
            let name = section.split([']', ' ', '"']).next().unwrap_or("");
            in_include =
                name.eq_ignore_ascii_case("include") || name.eq_ignore_ascii_case("includeif");
            continue;
        }
        if !in_include {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if key.trim().eq_ignore_ascii_case("path") {
                let value = value.trim().trim_matches('"');
                if !value.is_empty() {
                    out.push(value.to_string());
                }
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "grants_tests.rs"]
mod tests;

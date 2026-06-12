use std::path::{Path, PathBuf};

use super::types::{SecurityPolicy, TrustedAccess, POLICY_BLOCKED_MARKER};
use super::types::{WORKSPACE_INTERNAL_DIRS, WORKSPACE_INTERNAL_FILES};

impl SecurityPolicy {
    /// Expand a leading `~/` to the user's home directory. Delegates to
    /// [`crate::openhuman::config::expand_tilde`] — the single source of truth —
    /// so policy and config expand paths byte-for-byte identically (and both
    /// produce platform-native separators; see issue #3353).
    pub(super) fn expand_tilde(&self, path: &str) -> String {
        crate::openhuman::config::expand_tilde(path)
    }

    /// String-only path check. Does NOT resolve symlinks.
    /// Use `validate_path()` for any path that will be used for file I/O.
    pub fn is_path_string_allowed(&self, path: &str) -> bool {
        // Block null bytes (can truncate paths in C-backed syscalls)
        if path.contains('\0') {
            return false;
        }

        // Block path traversal: check for ".." as a path component
        if Path::new(path)
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return false;
        }

        // Block URL-encoded traversal attempts (e.g. ..%2f)
        let lower = path.to_lowercase();
        if lower.contains("..%2f") || lower.contains("%2f..") {
            return false;
        }

        // Expand tilde for comparison
        let expanded = self.expand_tilde(path);
        let expanded_path = Path::new(&expanded);

        // Credential stores are never reachable, even via a trusted-root grant.
        if Self::is_always_forbidden(expanded_path) {
            return false;
        }

        // A trusted root grants access to its subtree, taking precedence over
        // workspace_only and forbidden_paths. Read-vs-write is enforced by the
        // operation-specific validators (validate_path / validate_parent_path).
        let in_trusted_root = self.is_within_trusted_root(expanded_path, false);

        // Block agent access to internal state paths under workspace_dir
        // (unless the path falls under an explicitly granted trusted root).
        if !in_trusted_root {
            let check = if expanded_path.is_absolute() {
                expanded_path.to_path_buf()
            } else {
                self.workspace_dir.join(expanded_path)
            };
            if self.is_workspace_internal_path(&check) {
                log::trace!(
                    "[security:policy] path blocked: agent access to workspace-internal state (requested={}, resolved={})",
                    path,
                    check.display()
                );
                return false;
            }
        }

        // Block absolute paths when workspace_only is set (unless trusted-rooted).
        if self.workspace_only && expanded_path.is_absolute() && !in_trusted_root {
            return false;
        }

        // Block forbidden paths using path-component-aware matching (unless trusted-rooted).
        if !in_trusted_root {
            for forbidden in &self.forbidden_paths {
                let forbidden_expanded = self.expand_tilde(forbidden);
                let forbidden_path = Path::new(&forbidden_expanded);
                if expanded_path.starts_with(forbidden_path) {
                    return false;
                }
            }
        }

        // Symlink-safe check (#1927). The string-level checks above can be
        // bypassed by creating a symlink inside the workspace that points to
        // a forbidden tree (e.g. `evil -> /etc/shadow`). Canonicalize the
        // path and re-validate `workspace_only` containment + forbidden_paths
        // against the resolved location.
        if let Some(canonical) = self.try_canonicalize_under_workspace(path) {
            if Self::is_always_forbidden(&canonical) {
                return false;
            }
            let workspace_root = self
                .workspace_dir
                .canonicalize()
                .unwrap_or_else(|_| self.workspace_dir.clone());
            let canonical_in_trusted = self.is_within_trusted_root(&canonical, false);
            if self.workspace_only
                && !canonical.starts_with(&workspace_root)
                && !canonical_in_trusted
            {
                log::trace!(
                    "[security:policy] path blocked: symlink escapes workspace (requested={}, resolved={}, workspace={})",
                    path,
                    canonical.display(),
                    workspace_root.display()
                );
                return false;
            }
            // If the resolved path stays inside the workspace, trust the
            // workspace boundary over forbidden_paths — otherwise a workspace
            // that lives under e.g. `/tmp` (common in tests and sandboxes)
            // would block every legitimate access. forbidden_paths is meant
            // to catch escapes *outside* the workspace, which the workspace
            // containment check above already validates.
            let inside_workspace = canonical.starts_with(&workspace_root);
            if !inside_workspace && !canonical_in_trusted {
                for forbidden in &self.forbidden_paths {
                    let forbidden_expanded = if let Some(stripped) = forbidden.strip_prefix("~/") {
                        std::env::var("HOME")
                            .ok()
                            .map(|h| PathBuf::from(h).join(stripped))
                            .unwrap_or_else(|| PathBuf::from(forbidden))
                    } else {
                        PathBuf::from(forbidden)
                    };
                    let forbidden_canonical = forbidden_expanded
                        .canonicalize()
                        .unwrap_or(forbidden_expanded);
                    if canonical.starts_with(&forbidden_canonical) {
                        log::trace!(
                        "[security:policy] path blocked: symlink resolves to forbidden tree (requested={}, resolved={}, forbidden={})",
                        path,
                        canonical.display(),
                        forbidden_canonical.display()
                    );
                        return false;
                    }
                }
            }
        }

        true
    }

    /// Resolve a user-supplied path under the workspace, canonicalizing it
    /// (or its parent) when present on disk. Used by [`Self::is_path_string_allowed`]
    /// to defend against symlink-based escapes that pass the string-level
    /// checks. Returns `None` only when neither the path nor its parent can
    /// be resolved on disk — in that case the caller falls back to the
    /// string-level checks alone (which is the safe default for fresh paths
    /// whose entire chain does not yet exist).
    fn try_canonicalize_under_workspace(&self, path: &str) -> Option<PathBuf> {
        let expanded = if let Some(stripped) = path.strip_prefix("~/") {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(stripped))?
        } else {
            PathBuf::from(path)
        };
        let absolute = if expanded.is_absolute() {
            expanded
        } else {
            self.workspace_dir.join(&expanded)
        };
        if let Ok(canonical) = absolute.canonicalize() {
            return Some(canonical);
        }
        // Path itself does not exist (e.g. a write-to-new-file call). Try
        // canonicalizing the parent + appending the basename so we still
        // catch parent chains that resolve via symlink to a forbidden tree.
        let parent = absolute.parent()?;
        let name = absolute.file_name()?;
        parent.canonicalize().ok().map(|p| p.join(name))
    }

    /// Return the canonical form of `workspace_dir`, hydrating the
    /// `canonical_workspace` cache on the first call.
    ///
    /// `validate_path` / `validate_parent_path` both need the canonical
    /// workspace root for forbidden-path containment checks. The underlying
    /// `tokio::fs::canonicalize` is a `stat(2)` + symlink walk and was
    /// previously invoked on every call with the same input.
    ///
    /// Falls back to the raw `workspace_dir` if `canonicalize` fails (e.g.
    /// during early startup or in tests where the workspace doesn't exist on
    /// disk), matching the inline behavior the callers used before the cache.
    pub(super) async fn workspace_root(&self) -> PathBuf {
        self.canonical_workspace
            .get_or_init(|| async {
                tokio::fs::canonicalize(&self.workspace_dir)
                    .await
                    .unwrap_or_else(|_| self.workspace_dir.clone())
            })
            .await
            .clone()
    }

    /// Validate a path for file I/O: string checks, canonicalize, workspace containment,
    /// and forbidden-path check on the resolved path.
    /// Returns the canonical `PathBuf` on success.
    pub async fn validate_path(&self, path: &str) -> Result<PathBuf, String> {
        if !self.is_path_string_allowed(path) {
            return Err(format!(
                "{POLICY_BLOCKED_MARKER} Path not allowed by security policy: {path}. Do not \
                 retry this path; use an allowed location (the workspace or a granted folder)."
            ));
        }
        let expanded = self.expand_tilde(path);
        let full_path = if Path::new(&expanded).is_absolute() {
            PathBuf::from(&expanded)
        } else {
            self.action_dir.join(&expanded)
        };
        let resolved = tokio::fs::canonicalize(&full_path)
            .await
            .map_err(|e| format!("Failed to resolve path '{path}': {e}"))?;
        if !self.is_resolved_path_allowed_for(&resolved, false) {
            return Err(format!(
                "{POLICY_BLOCKED_MARKER} Resolved path escapes workspace: {}",
                resolved.display()
            ));
        }
        let workspace_root = self.workspace_root().await;
        self.check_resolved_against_forbidden(&resolved, &workspace_root)?;
        log::debug!(
            "[security] validate_path: '{}' resolved to '{}'",
            path,
            resolved.display()
        );
        Ok(resolved)
    }

    /// Like `validate_path` but canonicalizes the parent directory.
    /// Use for write operations where the target file may not yet exist.
    /// Does NOT require the parent directory to exist — walks up to the deepest
    /// existing ancestor and checks that for symlink escapes.
    /// Returns the canonical full path (parent resolved + filename appended).
    pub async fn validate_parent_path(&self, path: &str) -> Result<PathBuf, String> {
        if !self.is_path_string_allowed(path) {
            return Err(format!(
                "{POLICY_BLOCKED_MARKER} Path not allowed by security policy: {path}. Do not \
                 retry this path; use an allowed location (the workspace or a granted folder)."
            ));
        }
        let expanded = self.expand_tilde(path);
        let full_path = if Path::new(&expanded).is_absolute() {
            PathBuf::from(&expanded)
        } else {
            self.action_dir.join(&expanded)
        };
        let parent = full_path
            .parent()
            .ok_or_else(|| format!("Invalid path (no parent): {path}"))?;
        let file_name = full_path
            .file_name()
            .ok_or_else(|| format!("Invalid path (no filename): {path}"))?;

        // Walk up to the deepest existing ancestor so we can canonicalize without
        // requiring the full parent path to exist yet. This catches symlink escapes
        // in existing path components even when deeper dirs are not created yet.
        let mut existing_ancestor = parent.to_path_buf();
        loop {
            if existing_ancestor.exists() {
                break;
            }
            match existing_ancestor.parent() {
                Some(p) => existing_ancestor = p.to_path_buf(),
                None => break,
            }
        }
        let canonical_ancestor = tokio::fs::canonicalize(&existing_ancestor)
            .await
            .map_err(|e| format!("Failed to resolve parent of '{path}': {e}"))?;
        if !self.is_resolved_path_allowed_for(&canonical_ancestor, true) {
            return Err(format!(
                "{POLICY_BLOCKED_MARKER} Resolved parent path escapes workspace: {}",
                canonical_ancestor.display()
            ));
        }

        // Build resolved result: canonical_ancestor + suffix from existing_ancestor to parent + filename.
        // Since is_path_string_allowed blocked "..", all components between the ancestor
        // and the intended parent are newly created dirs — no symlinks possible there.
        let relative_suffix = parent
            .strip_prefix(&existing_ancestor)
            .unwrap_or(std::path::Path::new(""));
        let resolved_parent = canonical_ancestor.join(relative_suffix);
        let result = resolved_parent.join(file_name);

        let workspace_root = self.workspace_root().await;
        self.check_resolved_against_forbidden(&canonical_ancestor, &workspace_root)?;
        self.check_resolved_against_forbidden(&result, &workspace_root)?;

        log::debug!(
            "[security] validate_parent_path: '{}' resolved parent to '{}'",
            path,
            resolved_parent.display()
        );
        Ok(result)
    }

    /// Returns `true` if `path` falls under one of the internal-state
    /// subdirectories or files within `workspace_dir`. Agent tools must not
    /// write to these locations — they contain memory DBs, session transcripts,
    /// tokens, and other core persistence that is not part of the agent's
    /// action surface.
    pub fn is_workspace_internal_path(&self, path: &Path) -> bool {
        // Try canonical forms first (handles symlinks), fall back to raw paths
        // when they don't exist on disk yet.
        let ws_canonical = self.workspace_dir.canonicalize();
        let path_canonical = path.canonicalize();
        let (ws, check_path) = match (&ws_canonical, &path_canonical) {
            (Ok(w), Ok(p)) => (w.as_path(), p.as_path()),
            _ => (self.workspace_dir.as_path(), path),
        };
        if !check_path.starts_with(ws) {
            return false;
        }
        let relative = match check_path.strip_prefix(ws) {
            Ok(r) => r,
            Err(_) => return false,
        };
        let first_component = match relative.components().next() {
            Some(std::path::Component::Normal(s)) => s.to_string_lossy(),
            _ => return false,
        };
        if WORKSPACE_INTERNAL_DIRS
            .iter()
            .any(|d| *d == first_component.as_ref())
        {
            return true;
        }
        // Check single-file entries (only if the relative path is exactly one component)
        if relative.components().count() == 1
            && WORKSPACE_INTERNAL_FILES
                .iter()
                .any(|f| *f == first_component.as_ref())
        {
            return true;
        }
        false
    }

    /// Paths that remain blocked even when a `trusted_root` grant would
    /// otherwise reach them — credential stores and core OS directories. A
    /// grant on a parent must never expose SSH/GPG/AWS/keychain secrets, nor
    /// open `/etc`, `C:\Windows`, `/System`, etc. Matching is **case-insensitive**
    /// (Windows/macOS filesystems are), so `.SSH` / `C:\WINDOWS` cannot slip
    /// through. Gray-area dirs (`/usr`, `/opt`, `/var`, `~/Library`) stay in the
    /// user-overridable `forbidden_paths` instead, so a grant can still reach
    /// e.g. `/usr/local/...`.
    pub(crate) fn is_always_forbidden(path: &Path) -> bool {
        // Normalize separators + case BEFORE splitting: a Windows backslash
        // path is a single component on POSIX (and vice-versa), so we segment
        // the normalized string rather than rely on `Path::components()`.
        let lc_path = path
            .to_string_lossy()
            .to_ascii_lowercase()
            .replace('\\', "/");
        let segments: Vec<&str> = lc_path.split('/').filter(|s| !s.is_empty()).collect();

        // (a) Credential stores — matched by path segment, location-independent
        // (catches e.g. `C:\Users\x\.ssh` and `~/Library/Keychains`).
        const SENSITIVE_COMPONENTS: &[&str] =
            &[".ssh", ".gnupg", ".aws", ".azure", ".kube", "keychains"];
        if segments.iter().any(|s| SENSITIVE_COMPONENTS.contains(s)) {
            return true;
        }
        // Windows DPAPI / credential stores live under `…\Microsoft\{Protect,
        // Credentials,Crypto,Vault}` — match the pair so the generic second
        // name can't false-positive an unrelated project directory.
        if segments.windows(2).any(|w| {
            w[0] == "microsoft" && matches!(w[1], "protect" | "credentials" | "crypto" | "vault")
        }) {
            return true;
        }

        // (b) Core OS directories — matched by absolute prefix. Unconditional,
        // unlike the user-overridable `forbidden_paths`.
        const SYSTEM_PREFIXES: &[&str] = &[
            // POSIX
            "/etc",
            "/root",
            "/boot",
            "/proc",
            "/sys",
            // macOS (note: /private is intentionally NOT blocked — macOS temp
            // dirs and /etc canonicalize under /private/var and /private/etc).
            "/system",
            // Windows
            "c:/windows",
            "c:/program files",
            "c:/program files (x86)",
            "c:/programdata",
        ];
        SYSTEM_PREFIXES
            .iter()
            .any(|p| lc_path == *p || lc_path.starts_with(&format!("{p}/")))
    }

    /// True if `path` is within a configured trusted root. When `require_write`
    /// is set, only `ReadWrite` roots match. Never matches credential stores.
    pub fn is_within_trusted_root(&self, path: &Path, require_write: bool) -> bool {
        if Self::is_always_forbidden(path) {
            return false;
        }
        self.trusted_roots.iter().any(|root| {
            if require_write && root.access != TrustedAccess::ReadWrite {
                return false;
            }
            let root_path = PathBuf::from(self.expand_tilde(&root.path));
            let canonical_root = root_path
                .canonicalize()
                .unwrap_or_else(|_| root_path.clone());
            path.starts_with(&root_path) || path.starts_with(&canonical_root)
        })
    }

    /// Validate that a resolved path is still inside the workspace.
    /// Call this AFTER joining `workspace_dir` + relative path and canonicalizing.
    pub fn is_resolved_path_allowed(&self, resolved: &Path) -> bool {
        self.is_resolved_path_allowed_for(resolved, false)
    }

    /// Operation-aware resolved-path check: allowed when under the workspace, or
    /// within a trusted root (write roots only when `require_write`). Prefers the
    /// canonical workspace root so `/a/../b` style config paths don't misfire.
    pub fn is_resolved_path_allowed_for(&self, resolved: &Path, require_write: bool) -> bool {
        if Self::is_always_forbidden(resolved) {
            return false;
        }
        let workspace_root = self
            .workspace_dir
            .canonicalize()
            .unwrap_or_else(|_| self.workspace_dir.clone());
        resolved.starts_with(&workspace_root)
            || self.is_within_trusted_root(resolved, require_write)
    }

    /// Check `resolved` against every entry in `forbidden_paths`, resolving relative
    /// entries against `workspace_root`. Absolute entries whose prefix IS the workspace
    /// root are skipped — the workspace containment check already covers them.
    pub(super) fn check_resolved_against_forbidden(
        &self,
        resolved: &Path,
        workspace_root: &Path,
    ) -> Result<(), String> {
        // Credential stores are never reachable, even via a trusted-root grant.
        if Self::is_always_forbidden(resolved) {
            return Err(format!(
                "{POLICY_BLOCKED_MARKER} Resolved path is a protected credential store: {}",
                resolved.display()
            ));
        }
        // A trusted-root grant takes precedence over forbidden_paths for its subtree.
        if self.is_within_trusted_root(resolved, false) {
            return Ok(());
        }
        for forbidden in &self.forbidden_paths {
            let forbidden_path = PathBuf::from(self.expand_tilde(forbidden));
            let forbidden_resolved = if forbidden_path.is_absolute() {
                if workspace_root.starts_with(&forbidden_path) {
                    continue;
                }
                forbidden_path
            } else {
                workspace_root.join(forbidden_path)
            };
            if resolved.starts_with(&forbidden_resolved) {
                return Err(format!(
                    "{POLICY_BLOCKED_MARKER} Resolved path is inside a forbidden directory: {}",
                    forbidden_resolved.display()
                ));
            }
        }
        Ok(())
    }
}

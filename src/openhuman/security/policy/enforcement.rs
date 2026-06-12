use std::path::Path;

use super::types::{
    ActionTracker, AutonomyLevel, SecurityPolicy, ToolOperation, TrustedAccess, TrustedRoot,
    POLICY_BLOCKED_MARKER,
};
use std::sync::Arc;
use tokio::sync::OnceCell;

impl SecurityPolicy {
    /// Check if autonomy level permits any action at all
    pub fn can_act(&self) -> bool {
        self.autonomy != AutonomyLevel::ReadOnly
    }

    /// Enforce policy for a tool operation.
    ///
    /// Read operations are always allowed by autonomy/rate gates.
    /// Act operations require non-readonly autonomy and available action budget.
    pub fn enforce_tool_operation(
        &self,
        operation: ToolOperation,
        operation_name: &str,
    ) -> Result<(), String> {
        match operation {
            ToolOperation::Read => Ok(()),
            ToolOperation::Act => {
                if !self.can_act() {
                    log::warn!(
                        "[openhuman:policy] Operation '{}' blocked: read-only mode",
                        operation_name
                    );
                    return Err(format!(
                        "{POLICY_BLOCKED_MARKER} Security policy: read-only mode, cannot perform \
                         '{operation_name}'. Do not retry; this tier blocks all write actions."
                    ));
                }

                if !self.record_action() {
                    log::warn!(
                        "[openhuman:policy] Operation '{}' blocked: rate limit exceeded",
                        operation_name
                    );
                    return Err(format!(
                        "Rate limit exceeded: action budget exhausted ({} actions/hour). Increase the limit in Settings -> Advanced -> Agent autonomy or wait for the rolling one-hour window to refill.",
                        self.max_actions_per_hour
                    ));
                }

                log::debug!(
                    "[openhuman:policy] Operation '{}' allowed (actions: {}/{})",
                    operation_name,
                    self.tracker.count(),
                    self.max_actions_per_hour
                );
                Ok(())
            }
        }
    }

    /// Record an action and check if the rate limit has been exceeded.
    /// Returns `true` if the action is allowed, `false` if rate-limited.
    pub fn record_action(&self) -> bool {
        let count = self.tracker.record();
        count <= self.max_actions_per_hour as usize
    }

    /// Check if the rate limit would be exceeded without recording.
    pub fn is_rate_limited(&self) -> bool {
        self.tracker.count() >= self.max_actions_per_hour as usize
    }

    /// Build from config sections
    pub fn from_config(
        autonomy_config: &crate::openhuman::config::AutonomyConfig,
        workspace_dir: &Path,
        action_dir: &Path,
    ) -> Self {
        log::info!(
            "[openhuman:policy] SecurityPolicy created: autonomy={:?}, workspace_only={}, allowed_cmds={}, max_actions/hr={}",
            autonomy_config.level,
            autonomy_config.workspace_only,
            autonomy_config.allowed_commands.len(),
            autonomy_config.max_actions_per_hour
        );

        // `auto_approve` is the user's "Always allow" allowlist: the
        // `ApprovalGate` reads it via `live_policy::current()` and skips the
        // interactive prompt for any tool named in it. Tier + `CommandClass`
        // (and the unconditional read-only / forbidden-path / high-risk denials)
        // still run *before* the gate, so the allowlist can only suppress the
        // human prompt — it can never override a hard policy denial.

        // The default projects home (`~/OpenHuman/projects`) is always a
        // read-write trusted root so the coding agent can create/edit projects
        // there regardless of tier or `workspace_only`. Injected here — the one
        // autonomy→policy chokepoint every session goes through — because the
        // channels-startup injection is skipped on cores with no listening
        // integrations (web-chat-only), and a freshly reloaded config wouldn't
        // carry an in-memory edit anyway. A user-granted entry is left as-is.
        let mut trusted_roots = autonomy_config.trusted_roots.clone();
        let projects_path = crate::openhuman::config::default_projects_dir()
            .to_string_lossy()
            .to_string();
        if !trusted_roots.iter().any(|r| r.path == projects_path) {
            trusted_roots.push(TrustedRoot {
                path: projects_path,
                access: TrustedAccess::ReadWrite,
            });
        }

        // Dedicated, namespaced scratch dir (`/tmp/openhuman`) granted ReadWrite
        // so the LLM's natural `/tmp/...` temp-file habit lands in a sandboxed,
        // trusted location instead of the world-shared `/tmp`. Only this subdir
        // is ever trusted — never `/tmp` itself. Created here with restrictive
        // perms and refused if it exists as a symlink (TOCTOU hardening, since
        // `/tmp` is world-writable and the name is predictable).
        match ensure_openhuman_scratch_dir() {
            Some(scratch) => {
                let scratch_str = scratch.to_string_lossy().to_string();
                if trusted_roots.iter().any(|r| r.path == scratch_str) {
                    tracing::debug!(
                        path = %scratch_str,
                        "[policy][scratch] scratch dir already a trusted root — skipping grant"
                    );
                } else {
                    tracing::debug!(
                        path = %scratch_str,
                        "[policy][scratch] granting scratch dir as ReadWrite trusted root"
                    );
                    trusted_roots.push(TrustedRoot {
                        path: scratch_str,
                        access: TrustedAccess::ReadWrite,
                    });
                }
            }
            None => {
                tracing::debug!(
                    "[policy][scratch] scratch dir unavailable (create/validation failed) — not granting"
                );
            }
        }

        Self {
            autonomy: autonomy_config.level,
            workspace_dir: workspace_dir.to_path_buf(),
            action_dir: action_dir.to_path_buf(),
            workspace_only: autonomy_config.workspace_only,
            allowed_commands: autonomy_config.allowed_commands.clone(),
            forbidden_paths: autonomy_config.forbidden_paths.clone(),
            max_actions_per_hour: autonomy_config.max_actions_per_hour,
            max_cost_per_day_cents: autonomy_config.max_cost_per_day_cents,
            require_approval_for_medium_risk: autonomy_config.require_approval_for_medium_risk,
            block_high_risk_commands: autonomy_config.block_high_risk_commands,
            trusted_roots,
            allow_tool_install: autonomy_config.allow_tool_install,
            auto_approve: autonomy_config.auto_approve.clone(),
            tracker: ActionTracker::new(),
            canonical_workspace: Arc::new(OnceCell::new()),
        }
    }
}

/// The dedicated, namespaced scratch directory granted to the agent so its
/// natural `/tmp/...` temp-file habit lands in a sandboxed, trusted location
/// rather than the world-shared `/tmp`. Only this subdir is ever trusted —
/// never `/tmp` itself. On Windows, falls back to the per-user temp dir.
pub fn openhuman_scratch_dir() -> std::path::PathBuf {
    #[cfg(windows)]
    {
        std::env::temp_dir().join("openhuman")
    }
    #[cfg(not(windows))]
    {
        std::path::PathBuf::from("/tmp/openhuman")
    }
}

/// Create [`openhuman_scratch_dir`] with restrictive perms, best-effort.
/// Returns `None` (and grants nothing) if the path already exists as a
/// symlink — TOCTOU hardening, since the parent `/tmp` is world-writable and
/// the name is predictable. Idempotent: safe to call on every policy build.
pub fn ensure_openhuman_scratch_dir() -> Option<std::path::PathBuf> {
    let dir = openhuman_scratch_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(path = %dir.display(), error = %e, "[security][scratch] failed to create scratch dir");
        return None;
    }
    // Re-validate AFTER creation, and fail closed. `/tmp` is world-writable, so a
    // local user could have raced or pre-created `/tmp/openhuman` (e.g. as a
    // symlink) before we got here; a pre-create check alone can still hand back
    // an unsafe path. `symlink_metadata` does not follow the final component, so
    // a symlink is detected here even if its target is a real directory.
    let meta = match std::fs::symlink_metadata(&dir) {
        Ok(meta) => meta,
        Err(e) => {
            tracing::warn!(path = %dir.display(), error = %e, "[security][scratch] failed to stat scratch dir — refusing to grant");
            return None;
        }
    };
    if meta.file_type().is_symlink() || !meta.is_dir() {
        tracing::warn!(
            path = %dir.display(),
            "[security][scratch] scratch path is a symlink or not a directory — refusing to grant it as a trusted root"
        );
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)) {
            tracing::warn!(path = %dir.display(), error = %e, "[security][scratch] failed to harden scratch dir perms — refusing to grant");
            return None;
        }
    }
    tracing::debug!(path = %dir.display(), "[security][scratch] scratch dir ensured (0700, real dir)");
    Some(dir)
}

/// Validate that a file path resolves within a given root directory.
/// Canonicalizes both paths and checks that the resolved candidate
/// starts with the root. Callers should check `.is_file()` first
/// to avoid errors on non-existent paths (normal missing-file case).
///
/// Used to prevent path traversal in agent definition TOML files and
/// other user-controllable file references.
pub fn validate_path_within_root(
    candidate: &std::path::Path,
    root: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let resolved_root = root
        .canonicalize()
        .map_err(|e| format!("workspace root: {e}"))?;
    let resolved = candidate
        .canonicalize()
        .map_err(|e| format!("{}: {e}", candidate.display()))?;
    if !resolved.starts_with(&resolved_root) {
        return Err(format!(
            "path escapes root: {} is not under {}",
            resolved.display(),
            resolved_root.display()
        ));
    }
    Ok(resolved)
}

#[cfg(test)]
mod scratch_dir_tests {
    use super::{ensure_openhuman_scratch_dir, openhuman_scratch_dir};

    #[test]
    fn scratch_dir_is_namespaced_on_every_platform() {
        // Always the dedicated `openhuman` scratch namespace — never a bare
        // temp root, so only this subdir is ever granted as a trusted root.
        let dir = openhuman_scratch_dir();
        assert_eq!(dir.file_name().and_then(|s| s.to_str()), Some("openhuman"));
        #[cfg(not(windows))]
        assert_eq!(dir, std::path::PathBuf::from("/tmp/openhuman"));
    }

    #[test]
    fn ensure_scratch_dir_creates_and_returns_it() {
        // Idempotent: creates the dir, returns its path, and it exists after.
        let ensured = ensure_openhuman_scratch_dir();
        let expected = openhuman_scratch_dir();
        assert_eq!(ensured.as_deref(), Some(expected.as_path()));
        assert!(expected.is_dir());
    }
}

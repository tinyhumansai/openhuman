use std::path::Path;

#[cfg(feature = "security-module")]
use super::native_commands::{CommandFlow, CommandStage};
use super::types::{SecurityPolicy, ToolOperation, TrustedAccess, TrustedRoot};
use std::sync::Arc;
use tinytools::status::POLICY_BLOCKED_MARKER;

impl SecurityPolicy {
    /// Reserve one action from TinySecurity's hourly ledger.
    #[cfg(feature = "security-module")]
    pub async fn spend_action(&self, operation_name: &str) -> Result<(), String> {
        use tinysecurity_bus::BudgetDecision2;

        if !self.enabled {
            if let Ok(mut snapshot) = self.budget_snapshot.lock() {
                *snapshot = Some(BudgetDecision2::Allow);
            }
            return Ok(());
        }

        let policy = self
            .native_command_policy(CommandFlow::Harness, CommandStage::Execute)
            .map_err(|error| error.to_string())?;
        let decision = crate::modules::security::gap::budget(
            self.workspace_dir.clone(),
            policy,
            crate::modules::security::gap::BudgetOp::Spend,
        )
        .await
        .map_err(|error| {
            log::warn!("[security][policy] TinySecurity budget unavailable: {error}");
            format!(
                "{} TinySecurity action budget unavailable; action denied",
                tinytools::status::POLICY_BLOCKED_MARKER
            )
        })?;
        if let Ok(mut snapshot) = self.budget_snapshot.lock() {
            *snapshot = Some(decision.clone());
        }
        match decision {
            BudgetDecision2::Allow => Ok(()),
            BudgetDecision2::ReadOnlyMode => Err(format!(
                "{} Security policy: read-only mode, cannot perform '{operation_name}'. Do not retry; this tier blocks all write actions.",
                tinytools::status::POLICY_BLOCKED_MARKER
            )),
            BudgetDecision2::RateLimited { max_per_hour } => Err(format!(
                "Rate limit exceeded: action budget exhausted ({max_per_hour} actions/hour). Increase the limit in Settings -> Advanced -> Agent autonomy or wait for the rolling one-hour window to refill."
            )),
            _ => Err(format!(
                "{} TinySecurity returned an unknown action-budget decision",
                tinytools::status::POLICY_BLOCKED_MARKER
            )),
        }
    }

    /// Read the module-owned action budget without reserving an action.
    #[cfg(feature = "security-module")]
    pub async fn peek_action_budget(&self) -> Result<tinysecurity_bus::BudgetDecision2, String> {
        if !self.enabled {
            return Ok(tinysecurity_bus::BudgetDecision2::Allow);
        }
        let policy = self
            .native_command_policy(CommandFlow::Harness, CommandStage::Execute)
            .map_err(|error| error.to_string())?;
        let decision = crate::modules::security::gap::budget(
            self.workspace_dir.clone(),
            policy,
            crate::modules::security::gap::BudgetOp::Peek,
        )
        .await?;
        if let Ok(mut snapshot) = self.budget_snapshot.lock() {
            *snapshot = Some(decision.clone());
        }
        Ok(decision)
    }

    /// Without the module there is no budget authority, so the tier is unknown.
    #[cfg(not(feature = "security-module"))]
    pub async fn peek_action_budget(&self) -> Result<tinysecurity_bus::BudgetDecision2, String> {
        if !self.enabled {
            return Ok(tinysecurity_bus::BudgetDecision2::Allow);
        }
        Err("TinySecurity action budget unavailable".into())
    }

    /// Without the module there is no budget authority, so acting is refused.
    #[cfg(not(feature = "security-module"))]
    pub async fn spend_action(&self, _operation_name: &str) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        Err(format!(
            "{} TinySecurity action budget unavailable; action denied",
            tinytools::status::POLICY_BLOCKED_MARKER
        ))
    }

    /// Check if autonomy level permits any action at all.
    ///
    /// Always `true` when the policy is disabled ([`SecurityPolicy::enabled`]).
    pub fn can_act(&self) -> bool {
        if !self.enabled {
            return true;
        }
        let Ok(snapshot) = self.budget_snapshot.lock() else {
            return false;
        };
        matches!(
            snapshot.as_ref(),
            Some(tinysecurity_bus::BudgetDecision2::Allow)
                | Some(tinysecurity_bus::BudgetDecision2::RateLimited { .. })
        )
    }

    /// Cached class decision from TinySecurity for synchronous tool adapters.
    /// The async host gate refreshes it before the tool is dispatched; an
    /// enabled policy with no answer fails closed.
    pub fn gate_decision(&self, class: super::types::CommandClass) -> super::types::GateDecision {
        use super::types::GateDecision;
        if !self.enabled {
            return GateDecision::Allow;
        }
        let Ok(snapshot) = self.class_gate_snapshot.lock() else {
            return GateDecision::Block;
        };
        snapshot
            .iter()
            .rev()
            .find(|(cached, _)| *cached == class)
            .map(|(_, gate)| *gate)
            .unwrap_or(GateDecision::Block)
    }

    /// The **tier** half of an act check, without touching the hourly action
    /// budget.
    ///
    /// [`Self::enforce_tool_operation`]'s `Act` arm is tier *plus* budget, and
    /// that budget is denominated in agent *tool calls*. Callers that write on
    /// a caller's behalf at a finer grain than a tool call — the kernel memory
    /// guard, which sits under `MemoryCore::store` and is hit hundreds of times
    /// by one bulk ingest — want the tier refusal and nothing else. That is the
    /// same shape the ~15 acting tools which gate on bare [`Self::can_act`]
    /// already use (e.g. `tinyagents_harness_tools::fs::filesystem::FileWriteTool`,
    /// `cron/scheduler.rs`).
    pub fn enforce_write_tier(&self, operation_name: &str) -> Result<(), String> {
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
        Ok(())
    }

    /// Enforce policy for a tool operation.
    ///
    /// Read operations are always allowed by autonomy/rate gates.
    /// Act operations require non-readonly autonomy and available action budget.
    pub async fn enforce_tool_operation(
        &self,
        operation: ToolOperation,
        operation_name: &str,
    ) -> Result<(), String> {
        match operation {
            ToolOperation::Read => Ok(()),
            ToolOperation::Act => self.spend_action(operation_name).await,
        }
    }

    /// Compatibility read for TinyTools' synchronous gate trait.
    /// The async host authorization stage has already reserved the action in
    /// TinySecurity and refreshed `budget_snapshot`; this method keeps no
    /// local counter and never re-enters a Tokio runtime.
    pub fn record_action(&self) -> bool {
        if !self.enabled {
            return true;
        }
        !self.is_rate_limited()
    }

    /// Check if the rate limit would be exceeded without recording.
    pub fn is_rate_limited(&self) -> bool {
        if !self.enabled {
            return false;
        }
        let Ok(snapshot) = self.budget_snapshot.lock() else {
            return true;
        };
        match snapshot.as_ref() {
            Some(tinysecurity_bus::BudgetDecision2::RateLimited { .. }) | None => true,
            Some(tinysecurity_bus::BudgetDecision2::Allow)
            | Some(tinysecurity_bus::BudgetDecision2::ReadOnlyMode) => false,
            Some(_) => true,
        }
    }

    /// Build from config sections
    pub fn from_config(
        autonomy_config: &crate::config::AutonomyConfig,
        workspace_dir: &Path,
        action_dir: &Path,
    ) -> Self {
        Self::from_config_with(
            crate::core::runtime::is_saas(),
            autonomy_config,
            workspace_dir,
            action_dir,
        )
    }

    /// [`from_config`](Self::from_config) with the process mode passed in, so
    /// the SaaS grants (no shared projects home, no shared `/tmp/openhuman`)
    /// are testable without the process-wide mode lock.
    pub(crate) fn from_config_with(
        saas: bool,
        autonomy_config: &crate::config::AutonomyConfig,
        workspace_dir: &Path,
        action_dir: &Path,
    ) -> Self {
        log::info!(
            "[openhuman:policy] SecurityPolicy created: autonomy={:?}, workspace_only={}, allowed_cmds={}, max_actions/hr={}, auto_approve_all={}",
            autonomy_config.level,
            autonomy_config.workspace_only,
            autonomy_config.allowed_commands.len(),
            autonomy_config.max_actions_per_hour,
            autonomy_config.auto_approve_all
        );
        if !autonomy_config.enabled {
            log::info!(
                "[openhuman:policy] autonomy policy DISABLED ([autonomy] enabled = false) — command classification, approvals, allowlist, action budget and workspace containment are inert. TinySecurity's protected-path floor remains active."
            );
        }

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
        // SaaS: the projects home and `/tmp/openhuman` are shared by every user
        // of the process, so neither is granted there.
        // A relative or empty root cannot be a usable grant, so drop it here too:
        // the turn prompt lists these roots as grants.
        let mut trusted_roots: Vec<TrustedRoot> = autonomy_config
            .trusted_roots
            .iter()
            .filter(|root| {
                let absolute = Path::new(&crate::config::expand_tilde(&root.path)).is_absolute();
                if !absolute {
                    log::warn!(
                        "[openhuman:policy] ignoring non-absolute trusted root {:?}",
                        root.path
                    );
                }
                absolute
            })
            .cloned()
            .collect();
        let projects_path = crate::config::default_projects_dir()
            .to_string_lossy()
            .to_string();
        if !saas && !trusted_roots.iter().any(|r| r.path == projects_path) {
            trusted_roots.push(TrustedRoot {
                path: projects_path,
                access: TrustedAccess::ReadWrite,
            });
        }

        // The configured action dir is the agent's working root — `validate_path`
        // joins every relative tool path onto it — but until now it was only the
        // *join* base, never an allow root. The permission came from the
        // `default_projects_dir()` grant above, which reads
        // `OPENHUMAN_PROJECTS_DIR` and knows nothing about
        // `OPENHUMAN_ACTION_DIR` / `action_dir_override`. On a stock install the
        // two coincide and the gap is invisible; change the working folder in
        // Settings and every file-tool write into it was refused with "Resolved
        // path escapes workspace" — for a path inside the directory the agent
        // was told to work in. Grant it here, the same chokepoint, deduplicated
        // the same way.
        //
        // Guarded: an action dir at or above `workspace_dir` would hand a
        // trusted root to the whole workspace, and
        // TinySecurity lets a trusted root override `forbidden_paths`, but still
        // applies protected-path and internal-state reservations. The
        // `forbidden_paths` bypass is not something a working directory should
        // buy, so skip the grant when it covers the workspace.
        // Path checks resolve tool paths against `action_dir` (absolute after
        // canonicalising), so a relative action dir is granted as its absolute
        // form; only an absolute trusted root matches anything.
        let action_root =
            std::path::absolute(action_dir).unwrap_or_else(|_| action_dir.to_path_buf());
        let action_path = action_root.to_string_lossy().to_string();
        let action_covers_workspace = workspace_dir.starts_with(&action_root);
        if action_path.is_empty() || action_covers_workspace {
            tracing::debug!(
                action_dir = %action_dir.display(),
                workspace_dir = %workspace_dir.display(),
                "[policy] not granting action_dir as a trusted root (empty, or an ancestor of workspace_dir)"
            );
        } else if !trusted_roots.iter().any(|r| r.path == action_path) {
            trusted_roots.push(TrustedRoot {
                path: action_path,
                access: TrustedAccess::ReadWrite,
            });
        }

        // Oversized tool outputs are saved to `<workspace_dir>/artifacts/tool-results`
        // (`tool_result_artifacts_dir`), outside the action dir, and the model is
        // handed the absolute path to page them back with `file_read`. Under
        // `workspace_only` an absolute path is refused unless a trusted root
        // covers it, so without this grant every pointer would be unreadable.
        // Read-only: the agent reads its own outputs back; it has no reason to
        // write there. TinySecurity reserves the rest of artifact storage.
        let tool_results = super::types::tool_result_artifacts_dir(workspace_dir)
            .to_string_lossy()
            .to_string();
        if !trusted_roots.iter().any(|r| r.path == tool_results) {
            trusted_roots.push(TrustedRoot {
                path: tool_results,
                access: TrustedAccess::Read,
            });
        }

        // Dedicated, namespaced scratch dir (`/tmp/openhuman`) granted ReadWrite
        // so the LLM's natural `/tmp/...` temp-file habit lands in a sandboxed,
        // trusted location instead of the world-shared `/tmp`. Only this subdir
        // is ever trusted — never `/tmp` itself. Created here with restrictive
        // perms and refused if it exists as a symlink (TOCTOU hardening, since
        // `/tmp` is world-writable and the name is predictable).
        match (!saas).then(ensure_openhuman_scratch_dir).flatten() {
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
            enabled: autonomy_config.enabled,
            autonomy: autonomy_config.level,
            // Privacy mode is not carried on `AutonomyConfig` (it lives in the
            // separate `[privacy]` block), and `from_config` has ~40 call sites
            // that only hold the autonomy block — so we default to `Standard`
            // here and let the install / reload chokepoints layer the real
            // `config.privacy.mode` on via [`with_privacy_mode`]. The live-policy
            // paths (`install` seeds from the built policy, `reload_from`
            // re-applies the stored mode, `reload_privacy` swaps it) keep the
            // effective enforcement mode correct without touching every caller.
            privacy_mode: crate::config::PrivacyMode::default(),
            workspace_dir: workspace_dir.to_path_buf(),
            action_dir: action_dir.to_path_buf(),
            workspace_only: autonomy_config.workspace_only,
            allowed_commands: autonomy_config.allowed_commands.clone(),
            forbidden_paths: autonomy_config.forbidden_paths.clone(),
            max_actions_per_hour: autonomy_config.max_actions_per_hour,
            require_approval_for_medium_risk: autonomy_config.require_approval_for_medium_risk,
            block_high_risk_commands: autonomy_config.block_high_risk_commands,
            trusted_roots,
            media_artifact_paths: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            media_files_roots: Arc::new(std::sync::Mutex::new(None)),
            allow_tool_install: autonomy_config.allow_tool_install,
            auto_approve: autonomy_config.auto_approve.clone(),
            auto_approve_all: autonomy_config.auto_approve_all,
            command_gateway: None,
            budget_snapshot: Arc::new(std::sync::Mutex::new(None)),
            class_gate_snapshot: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    /// Return a copy of this policy with `privacy_mode` set. Used by the live-
    /// policy install / reload chokepoints to layer `config.privacy.mode` onto a
    /// policy that [`from_config`](Self::from_config) built with the `Standard`
    /// default. Builder-style so call sites read as
    /// `SecurityPolicy::from_config(..).with_privacy_mode(config.privacy.mode)`.
    #[must_use]
    pub fn with_privacy_mode(mut self, privacy_mode: crate::config::PrivacyMode) -> Self {
        log::debug!(
            "[privacy][policy] privacy_mode set on SecurityPolicy: {:?} (was {:?})",
            privacy_mode,
            self.privacy_mode
        );
        self.privacy_mode = privacy_mode;
        self
    }

    /// The active data-egress posture (Privacy Mode). Read by the inference
    /// chokepoint to enforce local-only model routing; later slices (S4/S7) also
    /// branch on `Sensitive` here.
    pub fn privacy_mode(&self) -> crate::config::PrivacyMode {
        self.privacy_mode
    }
}

#[cfg(all(test, feature = "security-module"))]
#[path = "budget_module_tests.rs"]
mod budget_module_tests;

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
#[path = "enforcement_scratch_dir_tests_tests.rs"]
mod scratch_dir_tests;

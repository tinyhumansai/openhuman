//! Process-global, hot-swappable [`SecurityPolicy`].
//!
//! `SecurityPolicy` is otherwise built once per agent session (see
//! `channels::runtime::startup`) and shared immutably to every tool. That makes
//! a runtime change to the `[autonomy]` block (via `config.update_autonomy_settings`)
//! invisible until a fresh session starts. This module holds the *current*
//! policy in a process-global cell so that:
//!
//! - new sessions always [`install`] (and therefore read) the latest policy, and
//! - [`reload_from`] swaps the shared policy the moment the config is saved, so
//!   [`current`] reflects the new access mode immediately.
//!
//! A future change can have tools read [`current`] per-call for true mid-turn
//! hot-swap; today the swap is observed at the next session boundary, which
//! matches how permission-mode changes are conventionally applied between turns.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use super::SecurityPolicy;
use crate::openhuman::config::PrivacyMode;

struct LiveState {
    policy: RwLock<Arc<SecurityPolicy>>,
    workspace_dir: RwLock<PathBuf>,
    action_dir: RwLock<PathBuf>,
    /// Stored Privacy Mode so an autonomy-only [`reload_from`] preserves the
    /// active mode (autonomy config carries no privacy field) and a later
    /// [`reload_privacy`] can swap it without rebuilding from a full `Config`.
    privacy_mode: RwLock<PrivacyMode>,
    generation: AtomicU64,
}

static STATE: OnceLock<LiveState> = OnceLock::new();

/// Install `policy` as the process-global live policy and remember
/// `workspace_dir` so later reloads rebuild against the same workspace.
/// Idempotent: later calls overwrite the stored policy (e.g. a new session
/// starting with a freshly loaded config). Returns the same `Arc` for chaining.
pub fn install(
    policy: Arc<SecurityPolicy>,
    workspace_dir: PathBuf,
    action_dir: PathBuf,
) -> Arc<SecurityPolicy> {
    let state = STATE.get_or_init(|| LiveState {
        policy: RwLock::new(Arc::clone(&policy)),
        workspace_dir: RwLock::new(workspace_dir.clone()),
        action_dir: RwLock::new(action_dir.clone()),
        privacy_mode: RwLock::new(policy.privacy_mode),
        generation: AtomicU64::new(0),
    });
    if let Ok(mut guard) = state.policy.write() {
        *guard = Arc::clone(&policy);
    }
    if let Ok(mut guard) = state.workspace_dir.write() {
        *guard = workspace_dir;
    }
    if let Ok(mut guard) = state.action_dir.write() {
        *guard = action_dir;
    }
    // Seed the stored privacy mode from the installed policy so later
    // autonomy-only reloads preserve it. `get_or_init` only runs the closure on
    // first install, so re-seed here on every install too.
    if let Ok(mut guard) = state.privacy_mode.write() {
        *guard = policy.privacy_mode;
    }
    log::debug!(
        "[privacy][live_policy] installed policy with privacy_mode={:?}",
        policy.privacy_mode
    );
    policy
}

#[cfg(test)]
thread_local! {
    /// Test-only, **thread-scoped** [`PrivacyMode`] override. When set it wins
    /// over the process-global policy in [`current_privacy_mode`].
    ///
    /// The egress-enforcement gate (privacy epic S7, #4441) reads
    /// [`current_privacy_mode`] on every integration / network / composio /
    /// embedding call. The process-global live policy is shared across the whole
    /// test binary, so a test that installed `LocalOnly` into it would flake any
    /// *other* parallel test whose tool happens to read the mode during that
    /// window. This thread-local lets a single test exercise the gate under a
    /// specific mode WITHOUT mutating the shared global — `#[tokio::test]` runs on
    /// a current-thread runtime, so the tool's inline read observes the override
    /// on the same thread while sibling tests on their own threads are unaffected.
    static TEST_PRIVACY_MODE: std::cell::Cell<Option<PrivacyMode>> =
        const { std::cell::Cell::new(None) };
}

/// RAII guard that restores the previous thread-local privacy override on drop.
#[cfg(test)]
pub(crate) struct TestPrivacyGuard(Option<PrivacyMode>);

#[cfg(test)]
impl Drop for TestPrivacyGuard {
    fn drop(&mut self) {
        TEST_PRIVACY_MODE.with(|c| c.set(self.0));
    }
}

/// Override [`current_privacy_mode`] for the current thread until the returned
/// guard drops. Test-only; needs no `TEST_ENV_LOCK` because it never touches the
/// process-global policy.
#[cfg(test)]
pub(crate) fn test_privacy_scope(mode: PrivacyMode) -> TestPrivacyGuard {
    let prev = TEST_PRIVACY_MODE.with(|c| c.replace(Some(mode)));
    TestPrivacyGuard(prev)
}

/// The current live Privacy Mode, if a policy has been [`install`]ed. Falls back
/// to [`PrivacyMode::Standard`] when no policy is installed (e.g. a CLI
/// invocation that never started a session runtime) — i.e. no egress
/// restriction by default.
pub fn current_privacy_mode() -> PrivacyMode {
    // Test-only per-thread override (see `TEST_PRIVACY_MODE`) wins so a test can
    // drive the egress gate without mutating the shared process-global policy.
    #[cfg(test)]
    if let Some(mode) = TEST_PRIVACY_MODE.with(|c| c.get()) {
        return mode;
    }
    current().map(|p| p.privacy_mode).unwrap_or_default()
}

/// The current live policy, if one has been [`install`]ed this process.
pub fn current() -> Option<Arc<SecurityPolicy>> {
    STATE
        .get()
        .and_then(|s| s.policy.read().ok().map(|g| Arc::clone(&g)))
}

/// Reload counter — incremented on every [`reload_from`]. Observability/tests.
pub fn generation() -> u64 {
    STATE
        .get()
        .map_or(0, |s| s.generation.load(Ordering::Relaxed))
}

/// Swap in a new `action_dir` and rebuild the live policy around it,
/// bumping the generation counter. Used by
/// [`config_set_action_dir`](crate::openhuman::config::ops::set_action_dir)
/// (issue #3240) so a Settings-driven change of the agent's writable root
/// takes effect immediately instead of waiting for the next session.
///
/// Returns the new generation on success, or `Err` if no policy is
/// installed yet (typically a CLI-only invocation that never started a
/// session runtime).
pub fn update_action_dir(new_action_dir: PathBuf) -> Result<u64, String> {
    let Some(state) = STATE.get() else {
        return Err(
            "[security:live_policy] no policy installed yet — cannot update action_dir".into(),
        );
    };
    {
        let mut guard = state
            .action_dir
            .write()
            .map_err(|e| format!("[security:live_policy] action_dir lock poisoned: {e}"))?;
        *guard = new_action_dir.clone();
    }
    // Rebuild the policy by cloning the current one and swapping the
    // action_dir field. This preserves the entire autonomy + trusted_roots
    // + forbidden_paths state — the only thing changing is the sandbox root.
    let current_policy = state
        .policy
        .read()
        .map(|g| Arc::clone(&g))
        .map_err(|e| format!("[security:live_policy] policy lock poisoned: {e}"))?;
    let mut rebuilt: SecurityPolicy = (*current_policy).clone();
    rebuilt.action_dir = new_action_dir;
    {
        let mut guard = state
            .policy
            .write()
            .map_err(|e| format!("[security:live_policy] policy write lock poisoned: {e}"))?;
        *guard = Arc::new(rebuilt);
    }
    let gen = state.generation.fetch_add(1, Ordering::Relaxed) + 1;
    tracing::info!(
        generation = gen,
        "[security:live_policy] SecurityPolicy reloaded after action_dir change"
    );
    Ok(gen)
}

/// Rebuild the policy from `autonomy_config` against the stored workspace dir
/// and swap it in, bumping the generation counter. No-op if nothing has been
/// installed yet (e.g. a CLI invocation that never started a session runtime).
pub fn reload_from(autonomy_config: &crate::openhuman::config::AutonomyConfig) {
    let Some(state) = STATE.get() else {
        return;
    };
    let workspace = state
        .workspace_dir
        .read()
        .map(|g| g.clone())
        .unwrap_or_default();
    let action = state
        .action_dir
        .read()
        .map(|g| g.clone())
        .unwrap_or_default();
    // `from_config` builds with the `Standard` default; re-apply the stored
    // privacy mode so an autonomy-only change does not silently reset egress
    // posture (autonomy config carries no privacy field).
    let stored_privacy = state.privacy_mode.read().map(|g| *g).unwrap_or_default();
    let rebuilt = Arc::new(
        SecurityPolicy::from_config(autonomy_config, &workspace, &action)
            .with_privacy_mode(stored_privacy),
    );
    if let Ok(mut guard) = state.policy.write() {
        *guard = rebuilt;
    }
    let gen = state.generation.fetch_add(1, Ordering::Relaxed) + 1;
    tracing::info!(
        generation = gen,
        privacy_mode = ?stored_privacy,
        "[security:live_policy] SecurityPolicy reloaded after autonomy config change"
    );
}

/// Swap the active Privacy Mode on the process-global live policy and rebuild
/// the current policy around it, bumping the generation counter. Called by
/// `config.set_privacy_mode` (#4435) so a Settings-driven mode change takes
/// effect for the inference chokepoint immediately, without a core restart.
///
/// Clones the in-flight policy and swaps only `privacy_mode`, preserving every
/// other access setting (mirrors [`set_action_dir`]). Also updates the stored
/// mode so a subsequent [`reload_from`] keeps it. Returns the new generation, or
/// `Err` if no policy is installed yet (typically a CLI-only invocation).
pub fn reload_privacy(new_mode: PrivacyMode) -> Result<u64, String> {
    let Some(state) = STATE.get() else {
        return Err(
            "[security:live_policy] no policy installed yet — cannot update privacy_mode".into(),
        );
    };
    {
        let mut guard = state
            .privacy_mode
            .write()
            .map_err(|e| format!("[security:live_policy] privacy_mode lock poisoned: {e}"))?;
        *guard = new_mode;
    }
    let current_policy = state
        .policy
        .read()
        .map(|g| Arc::clone(&g))
        .map_err(|e| format!("[security:live_policy] policy lock poisoned: {e}"))?;
    let rebuilt = (*current_policy).clone().with_privacy_mode(new_mode);
    {
        let mut guard = state
            .policy
            .write()
            .map_err(|e| format!("[security:live_policy] policy write lock poisoned: {e}"))?;
        *guard = Arc::new(rebuilt);
    }
    let gen = state.generation.fetch_add(1, Ordering::Relaxed) + 1;
    tracing::info!(
        generation = gen,
        privacy_mode = ?new_mode,
        "[security:live_policy] SecurityPolicy reloaded after privacy mode change"
    );
    Ok(gen)
}

/// Swap the agent action sandbox root on the process-global live policy.
///
/// Updates the stored `action_dir` (so a subsequent [`reload_from`] keeps the
/// new root) and rebuilds the current policy with `new` as its `action_dir`,
/// bumping the generation counter. Unlike [`reload_from`], this does not need
/// the `[autonomy]` block: it clones the in-flight policy and swaps only the
/// action root, preserving every other access setting. No-op if nothing has
/// been [`install`]ed yet.
///
/// Used by `config::update_agent_paths` so a UI-set action directory takes
/// effect for new sessions immediately, without a core restart.
pub fn set_action_dir(new: PathBuf) {
    let Some(state) = STATE.get() else {
        tracing::debug!(
            "[security:live_policy] set_action_dir called before install; no live policy to swap"
        );
        return;
    };

    if let Ok(mut guard) = state.action_dir.write() {
        *guard = new.clone();
    }

    let rebuilt = match state.policy.read() {
        Ok(current) => Some({
            let mut next = (**current).clone();
            next.action_dir = new.clone();
            Arc::new(next)
        }),
        Err(_) => {
            tracing::warn!(
                action_dir = %new.display(),
                "[security:live_policy] set_action_dir: policy read lock poisoned; \
                 action_dir stored but live policy not swapped — next reload_from will reconcile"
            );
            None
        }
    };

    if let Some(rebuilt) = rebuilt {
        if let Ok(mut guard) = state.policy.write() {
            *guard = rebuilt;
        } else {
            tracing::warn!(
                action_dir = %new.display(),
                "[security:live_policy] set_action_dir: policy write lock poisoned; \
                 rebuilt policy discarded — next reload_from will reconcile"
            );
        }
        let generation = state.generation.fetch_add(1, Ordering::Relaxed) + 1;
        tracing::info!(
            generation,
            action_dir = %new.display(),
            "[security:live_policy] SecurityPolicy action_dir swapped after agent-paths change"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::AutonomyConfig;
    use crate::openhuman::security::AutonomyLevel;

    #[test]
    fn install_then_reload_swaps_policy_and_bumps_generation() {
        // Serialize against other tests that install/reload this process-global
        // (the approval-gate auto_approve test and the autonomy `ops` tests),
        // which all take this same lock — otherwise a parallel install races.
        let _env = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let workspace = std::env::temp_dir().join("openhuman_live_policy_test");
        let initial = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: workspace.clone(),
            ..SecurityPolicy::default()
        });
        install(initial, workspace.clone(), workspace.clone());

        let before = generation();
        assert_eq!(
            current().expect("policy installed").autonomy,
            AutonomyLevel::Supervised
        );

        // Reload with a Full-access config and assert the swap is observed.
        let cfg = AutonomyConfig {
            level: AutonomyLevel::Full,
            workspace_only: false,
            ..AutonomyConfig::default()
        };
        reload_from(&cfg);

        assert!(generation() > before, "generation must increase on reload");
        assert_eq!(
            current().expect("policy still installed").autonomy,
            AutonomyLevel::Full
        );
    }

    #[test]
    fn reload_privacy_swaps_mode_and_survives_autonomy_reload() {
        // Same process-global lock as the other live-policy tests.
        let _env = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let workspace = std::env::temp_dir().join("openhuman_privacy_live_test");
        let initial = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            privacy_mode: PrivacyMode::Standard,
            workspace_dir: workspace.clone(),
            ..SecurityPolicy::default()
        });
        install(initial, workspace.clone(), workspace.clone());

        assert_eq!(current_privacy_mode(), PrivacyMode::Standard);

        // Swap to LocalOnly — the live policy reflects it immediately.
        let before = generation();
        reload_privacy(PrivacyMode::LocalOnly).expect("policy installed");
        assert!(generation() > before, "generation must increase");
        assert_eq!(current_privacy_mode(), PrivacyMode::LocalOnly);
        assert_eq!(
            current().expect("installed").privacy_mode,
            PrivacyMode::LocalOnly
        );

        // An autonomy-only reload must PRESERVE the privacy mode (autonomy
        // config carries no privacy field).
        let cfg = AutonomyConfig {
            level: AutonomyLevel::Full,
            ..AutonomyConfig::default()
        };
        reload_from(&cfg);
        assert_eq!(
            current().expect("installed").autonomy,
            AutonomyLevel::Full,
            "autonomy must update"
        );
        assert_eq!(
            current_privacy_mode(),
            PrivacyMode::LocalOnly,
            "privacy mode must survive an autonomy-only reload"
        );

        // Restore Standard before releasing the lock. The live policy is
        // process-global; since S7 (#4441) the egress-enforcement gate reads
        // `current_privacy_mode()` on every integration/network/composio/
        // embedding call, so leaving LocalOnly installed here would block sibling
        // mock-backend tests (which do not take TEST_ENV_LOCK) with a policy error.
        reload_privacy(PrivacyMode::Standard).expect("restore Standard");
    }

    #[test]
    fn set_action_dir_swaps_root_and_bumps_generation() {
        // Same process-global lock as the reload test — these install/swap the
        // shared live policy and would race each other otherwise.
        let _env = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let workspace = std::env::temp_dir().join("openhuman_set_action_dir_test_ws");
        let action = std::env::temp_dir().join("openhuman_set_action_dir_test_action_a");
        let initial = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Full,
            workspace_dir: workspace.clone(),
            action_dir: action.clone(),
            ..SecurityPolicy::default()
        });
        install(initial, workspace.clone(), action.clone());

        assert_eq!(
            current().expect("policy installed").action_dir,
            action,
            "precondition: action_dir starts at the installed value"
        );

        let before = generation();
        let new_action = std::env::temp_dir().join("openhuman_set_action_dir_test_action_b");
        set_action_dir(new_action.clone());

        assert!(
            generation() > before,
            "generation must increase on action_dir swap"
        );
        // A subsequent policy query reflects the new root...
        assert_eq!(
            current().expect("policy still installed").action_dir,
            new_action,
            "live policy must reflect the new action_dir"
        );
        // ...and unrelated access settings are preserved (not reset to default).
        assert_eq!(
            current().expect("policy still installed").autonomy,
            AutonomyLevel::Full,
            "autonomy level must survive an action_dir swap"
        );
        // The stored action_dir is updated so a later reload keeps the new root.
        let cfg = AutonomyConfig {
            level: AutonomyLevel::Full,
            ..AutonomyConfig::default()
        };
        reload_from(&cfg);
        assert_eq!(
            current().expect("policy still installed").action_dir,
            new_action,
            "reload after set_action_dir must keep the swapped root"
        );
    }
}

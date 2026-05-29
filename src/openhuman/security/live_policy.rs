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

struct LiveState {
    policy: RwLock<Arc<SecurityPolicy>>,
    workspace_dir: RwLock<PathBuf>,
    generation: AtomicU64,
}

static STATE: OnceLock<LiveState> = OnceLock::new();

/// Install `policy` as the process-global live policy and remember
/// `workspace_dir` so later reloads rebuild against the same workspace.
/// Idempotent: later calls overwrite the stored policy (e.g. a new session
/// starting with a freshly loaded config). Returns the same `Arc` for chaining.
pub fn install(policy: Arc<SecurityPolicy>, workspace_dir: PathBuf) -> Arc<SecurityPolicy> {
    let state = STATE.get_or_init(|| LiveState {
        policy: RwLock::new(Arc::clone(&policy)),
        workspace_dir: RwLock::new(workspace_dir.clone()),
        generation: AtomicU64::new(0),
    });
    if let Ok(mut guard) = state.policy.write() {
        *guard = Arc::clone(&policy);
    }
    if let Ok(mut guard) = state.workspace_dir.write() {
        *guard = workspace_dir;
    }
    policy
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
    let rebuilt = Arc::new(SecurityPolicy::from_config(autonomy_config, &workspace));
    if let Ok(mut guard) = state.policy.write() {
        *guard = rebuilt;
    }
    let gen = state.generation.fetch_add(1, Ordering::Relaxed) + 1;
    tracing::info!(
        generation = gen,
        "[security:live_policy] SecurityPolicy reloaded after autonomy config change"
    );
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
        install(initial, workspace.clone());

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
}

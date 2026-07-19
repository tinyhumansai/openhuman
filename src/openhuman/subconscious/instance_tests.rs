//! Runner mechanics driven by a scripted [`FakeProfile`] — the world-agnostic
//! tick behaviour (quiet short-circuit, baseline-on-success, rate-cap halt
//! lifecycle, supersede discard) verified without any real provider/agent IO.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::*;
use crate::openhuman::agent::turn_origin::TrustedAutomationSource;
use crate::openhuman::config::Config;
use crate::openhuman::subconscious::profile::{Observation, Reflection, SubconsciousProfile};

/// A scripted profile: records how often each stage ran and returns canned
/// observations / reflections so the runner mechanics can be asserted in
/// isolation.
struct FakeProfile {
    observations: Mutex<Vec<Observation>>,
    reflect_result: Mutex<Result<Reflection, String>>,
    observe_calls: AtomicUsize,
    prepare_calls: AtomicUsize,
    reflect_calls: AtomicUsize,
    commit_calls: AtomicUsize,
    /// When set, `reflect` bumps this generation counter once to simulate a
    /// newer tick superseding the in-flight one.
    supersede: Mutex<Option<Arc<AtomicU64>>>,
    supersede_pending: AtomicUsize,
}

impl FakeProfile {
    fn new(obs: Observation, reflect: Result<Reflection, String>) -> Self {
        Self {
            observations: Mutex::new(vec![obs]),
            reflect_result: Mutex::new(reflect),
            observe_calls: AtomicUsize::new(0),
            prepare_calls: AtomicUsize::new(0),
            reflect_calls: AtomicUsize::new(0),
            commit_calls: AtomicUsize::new(0),
            supersede: Mutex::new(None),
            supersede_pending: AtomicUsize::new(0),
        }
    }

    fn changed() -> Observation {
        Observation {
            rendered: "something changed".into(),
            has_changes: true,
            has_external_content: true,
            commit_token: None,
        }
    }

    fn quiet() -> Observation {
        Observation {
            rendered: String::new(),
            has_changes: false,
            has_external_content: false,
            commit_token: None,
        }
    }
}

#[async_trait::async_trait]
impl SubconsciousProfile for FakeProfile {
    fn id(&self) -> &'static str {
        "memory"
    }
    fn cadence(&self, _config: &Config) -> std::time::Duration {
        std::time::Duration::from_secs(300)
    }
    async fn observe(&self, _config: &Config) -> Observation {
        self.observe_calls.fetch_add(1, Ordering::SeqCst);
        let mut obs = self.observations.lock().unwrap();
        if obs.len() > 1 {
            obs.remove(0)
        } else {
            obs[0].clone()
        }
    }
    async fn prepare_context(&self, _config: &Config, _obs: &Observation) -> String {
        self.prepare_calls.fetch_add(1, Ordering::SeqCst);
        "prepared".into()
    }
    async fn reflect(
        &self,
        _config: &Config,
        _obs: &Observation,
        _prepared: &str,
    ) -> Result<Reflection, String> {
        self.reflect_calls.fetch_add(1, Ordering::SeqCst);
        if self.supersede_pending.swap(0, Ordering::SeqCst) > 0 {
            if let Some(gen) = self.supersede.lock().unwrap().as_ref() {
                gen.fetch_add(1, Ordering::SeqCst);
            }
        }
        self.reflect_result.lock().unwrap().clone()
    }
    async fn commit(&self, _config: &Config, _obs: &Observation) {
        self.commit_calls.fetch_add(1, Ordering::SeqCst);
    }
    fn origin(&self, obs: &Observation) -> TrustedAutomationSource {
        if obs.has_external_content {
            TrustedAutomationSource::SubconsciousTainted
        } else {
            TrustedAutomationSource::Subconscious
        }
    }
}

/// A config that routes the subconscious provider to an available BYO provider
/// with a stable, non-cloud signature (`other:groq`) so the provider gate never
/// short-circuits and the rate-cap signature is deterministic.
fn test_config(workspace: &std::path::Path) -> Config {
    let mut cfg = Config::default();
    cfg.workspace_dir = workspace.to_path_buf();
    cfg.subconscious_provider = Some("groq".to_string());
    cfg
}

fn build(profile: Arc<FakeProfile>, workspace: &std::path::Path) -> SubconsciousInstance {
    SubconsciousInstance::new(
        profile as Arc<dyn SubconsciousProfile>,
        workspace.to_path_buf(),
        true,
        5,
        "simple",
    )
}

// ── Rate-cap circuit breaker state machine (TAURI-RUST-HXF) ─────────────

#[test]
fn instance_state_rate_cap_transitions() {
    let mut state = InstanceState {
        last_tick_at: 0.0,
        total_ticks: 0,
        consecutive_failures: 0,
        provider_unavailable_reason: None,
        rate_cap_halt_signature: None,
    };
    let prefix = "[subconscious:memory]";

    // No halt armed → the tick proceeds (does not skip).
    assert!(!state.should_skip_for_rate_cap_halt("memory|other:groq", prefix));

    // A permanent rate-cap failure arms the halt + actionable reason.
    state.arm_rate_cap_halt("memory|other:groq", prefix);
    assert_eq!(
        state.rate_cap_halt_signature.as_deref(),
        Some("memory|other:groq")
    );
    assert_eq!(
        state.provider_unavailable_reason.as_deref(),
        Some(RATE_CAP_HALT_REASON)
    );

    // Same config still set → skip the doomed run, and count the skipped tick.
    let before = state.total_ticks;
    assert!(state.should_skip_for_rate_cap_halt("memory|other:groq", prefix));
    assert_eq!(state.total_ticks, before + 1);

    // User switched provider (signature changed) → clear halt + reason, resume.
    assert!(!state.should_skip_for_rate_cap_halt("memory|cloud", prefix));
    assert!(state.rate_cap_halt_signature.is_none());
    assert!(state.provider_unavailable_reason.is_none());
}

#[tokio::test]
async fn quiet_observation_commits_and_advances_without_reflecting() {
    let dir = tempfile::tempdir().unwrap();
    let profile = Arc::new(FakeProfile::new(FakeProfile::quiet(), Ok(Reflection::Idle)));
    let instance = build(profile.clone(), dir.path());

    let result = instance
        .run_tick_for_test(test_config(dir.path()))
        .await
        .unwrap();

    assert_eq!(profile.observe_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        profile.prepare_calls.load(Ordering::SeqCst),
        0,
        "no prepare"
    );
    assert_eq!(
        profile.reflect_calls.load(Ordering::SeqCst),
        0,
        "no reflect"
    );
    assert_eq!(profile.commit_calls.load(Ordering::SeqCst), 1, "committed");
    // last_tick_at advanced to this tick.
    let status = instance.status().await;
    assert!(status.last_tick_at.is_some());
    assert_eq!(status.consecutive_failures, 0);
    assert_eq!(status.total_ticks, 1);
    assert_eq!(result.response_chars, 0);
}

#[tokio::test]
async fn changed_observation_runs_full_pipeline_and_commits_on_success() {
    let dir = tempfile::tempdir().unwrap();
    let profile = Arc::new(FakeProfile::new(
        FakeProfile::changed(),
        Ok(Reflection::Acted { response_chars: 42 }),
    ));
    let instance = build(profile.clone(), dir.path());

    let result = instance
        .run_tick_for_test(test_config(dir.path()))
        .await
        .unwrap();

    assert_eq!(profile.observe_calls.load(Ordering::SeqCst), 1);
    assert_eq!(profile.prepare_calls.load(Ordering::SeqCst), 1);
    assert_eq!(profile.reflect_calls.load(Ordering::SeqCst), 1);
    assert_eq!(profile.commit_calls.load(Ordering::SeqCst), 1);
    assert_eq!(result.response_chars, 42);
    assert_eq!(instance.status().await.consecutive_failures, 0);
}

#[tokio::test]
async fn failing_reflect_holds_baseline_and_bumps_failures() {
    let dir = tempfile::tempdir().unwrap();
    let profile = Arc::new(FakeProfile::new(
        FakeProfile::changed(),
        Err("agent run: transient boom".into()),
    ));
    let instance = build(profile.clone(), dir.path());

    instance
        .run_tick_for_test(test_config(dir.path()))
        .await
        .unwrap();

    assert_eq!(profile.reflect_calls.load(Ordering::SeqCst), 1);
    assert_eq!(profile.commit_calls.load(Ordering::SeqCst), 0, "no commit");
    let status = instance.status().await;
    assert_eq!(status.consecutive_failures, 1);
    // A failed reflect must not advance last_tick_at (re-diff the window next).
    assert!(status.last_tick_at.is_none());
}

#[tokio::test]
async fn permanent_rate_cap_error_arms_halt_then_config_change_resumes() {
    let dir = tempfile::tempdir().unwrap();
    let rate_cap = r#"agent run: groq API error (413 Payload Too Large): {"error":{"message":"Request too large for model in organization on tokens per minute (TPM): Limit 8000, Requested 42084."}}"#;
    let profile = Arc::new(FakeProfile::new(
        FakeProfile::changed(),
        Err(rate_cap.to_string()),
    ));
    let instance = build(profile.clone(), dir.path());

    // Tick 1: reflect fails with a permanent 413 → halt armed under `memory|other:groq`.
    instance
        .run_tick_for_test(test_config(dir.path()))
        .await
        .unwrap();
    assert_eq!(profile.observe_calls.load(Ordering::SeqCst), 1);
    let status = instance.status().await;
    assert!(!status.provider_available, "halt surfaced as unavailable");

    // Tick 2: same config → the gate skips before observe even runs.
    instance
        .run_tick_for_test(test_config(dir.path()))
        .await
        .unwrap();
    assert_eq!(
        profile.observe_calls.load(Ordering::SeqCst),
        1,
        "halted tick must not observe"
    );

    // Tick 3: user switched provider (signature changes) → halt clears, resumes.
    let mut resumed = test_config(dir.path());
    resumed.subconscious_provider = Some("openai".to_string());
    instance.run_tick_for_test(resumed).await.unwrap();
    assert_eq!(
        profile.observe_calls.load(Ordering::SeqCst),
        2,
        "resumed tick observes again"
    );
}

#[tokio::test]
async fn completed_ticks_leave_no_checkpoint_threads() {
    // Phase 6: each tick uses a unique checkpoint thread; a completed tick GCs
    // it, so the checkpoint db stays bounded no matter how many ticks run.
    use tinyagents::graph::{Checkpointer, SqliteCheckpointer};

    let dir = tempfile::tempdir().unwrap();
    let profile = Arc::new(FakeProfile::new(
        FakeProfile::changed(),
        Ok(Reflection::Acted { response_chars: 1 }),
    ));
    let instance = build(profile.clone(), dir.path());

    for _ in 0..3 {
        instance
            .run_tick_for_test(test_config(dir.path()))
            .await
            .unwrap();
    }

    let db = dir.path().join("subconscious").join("graph_checkpoints.db");
    let cp = SqliteCheckpointer::<serde_json::Value>::open(&db).unwrap();
    let threads = cp.list_threads().await.unwrap();
    assert!(
        threads.is_empty(),
        "completed ticks pruned their threads, got {threads:?}"
    );
}

#[tokio::test]
async fn superseded_tick_discards_result_and_skips_commit() {
    let dir = tempfile::tempdir().unwrap();
    let profile = Arc::new(FakeProfile::new(
        FakeProfile::changed(),
        Ok(Reflection::Acted { response_chars: 99 }),
    ));
    let instance = build(profile.clone(), dir.path());
    // Wire the profile to bump the instance's generation during reflect.
    *profile.supersede.lock().unwrap() = Some(instance.generation_handle());
    profile.supersede_pending.store(1, Ordering::SeqCst);

    let result = instance
        .run_tick_for_test(test_config(dir.path()))
        .await
        .unwrap();

    assert_eq!(profile.reflect_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        profile.commit_calls.load(Ordering::SeqCst),
        0,
        "superseded tick must not commit"
    );
    assert_eq!(result.response_chars, 0, "result discarded");
    let status = instance.status().await;
    assert!(status.last_tick_at.is_none(), "no baseline advance");
    assert_eq!(
        instance.snapshot_failures().await,
        0,
        "not counted a failure"
    );
}

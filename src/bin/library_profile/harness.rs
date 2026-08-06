//! Shared profiling harness: hermetic fixture, RSS sampling, and the
//! `measure()` wrapper that every scenario runs its workload through.
//!
//! All diagnostics go to **stderr** (stdout must stay pure JSON) using the
//! stable `[library-profile]` prefix.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::platform::proc_metrics::{self, ProcSample, TreeSample};
use serde::Serialize;

/// One sampled point inside a measured workload. `delta_kib` is the RSS change
/// versus the *previous* checkpoint (or the baseline for the first one).
#[derive(Debug, Clone, Serialize)]
pub struct Checkpoint {
    pub label: String,
    pub at_ms: u128,
    pub rss_kib: u64,
    pub delta_kib: i64,
}

/// Turn wall-latency percentiles (milliseconds) collected under overlapping
/// load by the `fleet` scenario.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct TurnLatency {
    pub p50: u128,
    pub p95: u128,
    pub p99: u128,
    pub max: u128,
}

/// Per-descendant RSS entry in a [`TreeReport`].
#[derive(Debug, Clone, Serialize)]
pub struct TreeChild {
    pub name: String,
    pub rss_kib: u64,
}

/// Process-*tree* RSS reporting: this process plus every descendant
/// (interpreter children such as `node` / `python` a skill run spawns). Folds a
/// [`TreeSample`] down to the reported shape. `tree_rss_kib` counts self + all
/// descendants, so it exceeds `settled.rss_kib` whenever a child was live at
/// sample time.
#[derive(Debug, Clone, Serialize)]
pub struct TreeReport {
    pub tree_rss_kib: u64,
    pub child_count: usize,
    pub children: Vec<TreeChild>,
}

impl TreeReport {
    pub fn from_sample(sample: &TreeSample) -> Self {
        Self {
            tree_rss_kib: sample.tree_rss_kib,
            child_count: sample.children.len(),
            children: sample
                .children
                .iter()
                .map(|child| TreeChild {
                    name: child.name.clone(),
                    rss_kib: child.rss_kib,
                })
                .collect(),
        }
    }
}

/// Fleet capacity budget math (purely informational — scripts turn `fits` into
/// pass/fail). `projected_rss_mib_at_target = settled_base + marginal * target`.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct FleetBudget {
    pub target_agents: u64,
    pub ram_budget_mib: u64,
    pub projected_rss_mib_at_target: f64,
    pub fits: bool,
}

/// Pinned JSON output contract (schema_version = 2). New optional fields are
/// skipped when absent so the two original scenarios stay byte-identical.
#[derive(Debug, Serialize)]
pub struct ProfileResult {
    pub schema_version: u32,
    pub scenario: &'static str,
    pub workload_units: usize,
    pub duration_ms: u128,
    pub baseline: ProcSample,
    pub settled: ProcSample,
    pub peak_rss_kib: u64,
    pub retained_delta_kib: i64,
    pub peak_delta_kib: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turns: Option<usize>,
    /// (`fleet`) requested live-agent count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents: Option<usize>,
    /// (`fleet`) agents actually constructed (may be < `agents` on fd/OOM).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents_built: Option<usize>,
    /// (`fleet`) marginal RSS cost per agent (`baseline → constructed`), KiB.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marginal_rss_kib_per_agent: Option<f64>,
    /// (`fleet`) user+system CPU delta over the 10 s idle window, ms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_cpu_ms: Option<u64>,
    /// (`fleet`) turn wall-latency percentiles under overlapping load.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_latency_ms: Option<TurnLatency>,
    /// (`fleet`) capacity budget projection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<FleetBudget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoints: Option<Vec<Checkpoint>>,
    /// (`subagent-storm`) K parallel researcher subagents fanned out in the turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagents: Option<usize>,
    /// (`skill-run`) process-tree RSS including interpreter child processes.
    /// Reports the richest tree seen — the peak-during-workload sample when a
    /// short-lived child (e.g. `node`) has already exited by settle time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree: Option<TreeReport>,
    /// Present (and `true`) only when the dhat heap profiler is active, since
    /// RSS/time numbers are perturbed by dhat's global allocator.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dhat: Option<bool>,
}

/// Restores (or removes) an environment variable on drop.
pub struct EnvGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvGuard {
    pub fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

/// A hermetic config rooted in a throwaway temp workspace. Local inference,
/// Python, spaCy, and embeddings are all disabled so runs are offline.
pub struct Fixture {
    pub config: Config,
    _workspace_env: EnvGuard,
    _keyring_env: EnvGuard,
    _action_env: EnvGuard,
    _tmp: tempfile::TempDir,
}

pub fn fixture() -> Result<Fixture> {
    let tmp = tempfile::tempdir().context("create profile workspace")?;
    let root = tmp.path();
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace)?;
    // A real, writable action sandbox so acting tools (e.g. `node_exec`, which
    // spawns `node` in the action dir) have a valid cwd. Harmless for scenarios
    // that never act.
    let action_dir = root.join("action");
    std::fs::create_dir_all(&action_dir)?;

    let mut config_toml = r#"api_url = "http://127.0.0.1:9"
default_model = "profile-mock"
default_temperature = 0.0
chat_onboarding_completed = true

[secrets]
encrypt = false

[local_ai]
enabled = false
runtime_enabled = false

[runtime_python]
enabled = false

[memory_tree]
spacy_enabled = false
"#
    .to_string();
    if std::env::var_os("OPENHUMAN_PROFILE_DISABLE_MEMORY_WRITES").is_some() {
        config_toml.push_str(
            r#"
[memory]
auto_save = false

[learning]
episodic_capture_enabled = false
"#,
        );
    }
    // `skill-run` executes real acting tools (`node_exec`); those need the Full
    // autonomy tier so the write-class gate does not park the turn on approval.
    // Config::load_or_init inside the detached workflow run re-reads this file,
    // so the tier must live in config.toml (not just the in-memory Config).
    if std::env::var_os("OPENHUMAN_PROFILE_FULL_AUTONOMY").is_some() {
        config_toml.push_str(
            r#"
[autonomy]
level = "full"
"#,
        );
    }
    std::fs::write(root.join("config.toml"), &config_toml)?;

    let workspace_env = EnvGuard::set("OPENHUMAN_WORKSPACE", &root.to_string_lossy());
    let keyring_env = EnvGuard::set("OPENHUMAN_KEYRING_BACKEND", "file");
    let action_env = EnvGuard::set("OPENHUMAN_ACTION_DIR", &action_dir.to_string_lossy());
    let mut config: Config = toml::from_str(&config_toml)?;
    config.workspace_dir = workspace;
    config.action_dir = action_dir;
    config.memory_tree.embedding_endpoint = None;
    config.memory_tree.embedding_model = None;
    config.memory_tree.embedding_strict = false;

    Ok(Fixture {
        config,
        _workspace_env: workspace_env,
        _keyring_env: keyring_env,
        _action_env: action_env,
        _tmp: tmp,
    })
}

/// Background task that polls `proc_metrics::sample_self()` every 5 ms and
/// keeps the running peak RSS.
pub struct PeakSampler {
    peak: Arc<AtomicU64>,
    stop: Arc<AtomicU64>,
    task: tokio::task::JoinHandle<()>,
}

impl PeakSampler {
    pub fn start(initial_kib: u64) -> Self {
        let peak = Arc::new(AtomicU64::new(initial_kib));
        let stop = Arc::new(AtomicU64::new(0));
        let task_peak = Arc::clone(&peak);
        let task_stop = Arc::clone(&stop);
        let task = tokio::spawn(async move {
            while task_stop.load(Ordering::Relaxed) == 0 {
                if let Ok(sample) = proc_metrics::sample_self() {
                    task_peak.fetch_max(sample.rss_kib, Ordering::Relaxed);
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });
        Self { peak, stop, task }
    }

    pub async fn stop(self) -> u64 {
        self.stop.store(1, Ordering::Relaxed);
        let _ = self.task.await;
        self.peak.load(Ordering::Relaxed)
    }
}

/// Background task that polls `proc_metrics::sample_tree()` and keeps the
/// [`TreeSample`] with the highest `tree_rss_kib` seen — capturing the moment a
/// short-lived interpreter child (e.g. a `node -e` step) is resident, which a
/// settle-time-only sample would miss because the child has already exited.
pub struct TreePeakSampler {
    best: Arc<Mutex<Option<TreeSample>>>,
    stop: Arc<AtomicU64>,
    task: tokio::task::JoinHandle<()>,
}

impl TreePeakSampler {
    pub fn start() -> Self {
        let best: Arc<Mutex<Option<TreeSample>>> = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicU64::new(0));
        let task_best = Arc::clone(&best);
        let task_stop = Arc::clone(&stop);
        let task = tokio::spawn(async move {
            while task_stop.load(Ordering::Relaxed) == 0 {
                if let Ok(sample) = proc_metrics::sample_tree() {
                    let mut guard = task_best.lock().expect("tree peak lock");
                    let replace = guard
                        .as_ref()
                        .map(|prev| sample.tree_rss_kib > prev.tree_rss_kib)
                        .unwrap_or(true);
                    if replace {
                        *guard = Some(sample);
                    }
                }
                tokio::time::sleep(Duration::from_millis(15)).await;
            }
        });
        Self { best, stop, task }
    }

    pub async fn stop(self) -> Option<TreeSample> {
        self.stop.store(1, Ordering::Relaxed);
        let _ = self.task.await;
        self.best.lock().expect("tree peak lock").clone()
    }
}

/// Collects per-phase / per-turn checkpoints inside a measured workload.
/// Cloneable (shares one buffer) so it can be handed to closures freely.
#[derive(Clone)]
pub struct Recorder {
    inner: Arc<Mutex<RecorderState>>,
    started: Instant,
}

struct RecorderState {
    checkpoints: Vec<Checkpoint>,
    last_rss_kib: u64,
}

impl Recorder {
    fn new(baseline_kib: u64, started: Instant) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RecorderState {
                checkpoints: Vec::new(),
                last_rss_kib: baseline_kib,
            })),
            started,
        }
    }

    /// Sample RSS now and append a labelled checkpoint whose `delta_kib` is
    /// relative to the previous checkpoint (baseline for the first).
    pub fn checkpoint(&self, label: impl Into<String>) -> Result<()> {
        let label = label.into();
        let sample = proc_metrics::sample_self()?;
        let at_ms = self.started.elapsed().as_millis();
        let mut state = self.inner.lock().expect("recorder lock");
        let delta_kib = sample.rss_kib as i64 - state.last_rss_kib as i64;
        state.last_rss_kib = sample.rss_kib;
        eprintln!(
            "[library-profile] checkpoint label={label} at_ms={at_ms} rss_kib={} delta_kib={delta_kib}",
            sample.rss_kib
        );
        state.checkpoints.push(Checkpoint {
            label,
            at_ms,
            rss_kib: sample.rss_kib,
            delta_kib,
        });
        Ok(())
    }

    fn take(self) -> Vec<Checkpoint> {
        self.inner
            .lock()
            .expect("recorder lock")
            .checkpoints
            .clone()
    }
}

/// Run `workload` between a settled baseline and a settled post-run sample,
/// tracking peak RSS throughout. `turns` and any checkpoints pushed via the
/// `Recorder` are folded into the result (both `None`/omitted when unused).
pub async fn measure<F, Fut>(
    scenario: &'static str,
    workload_units: usize,
    turns: Option<usize>,
    workload: F,
) -> Result<ProfileResult>
where
    F: FnOnce(Recorder) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    measure_impl(scenario, workload_units, turns, false, workload).await
}

/// Like [`measure`], but also samples the process **tree** (self + descendant
/// interpreter processes). A [`TreePeakSampler`] runs alongside the RSS peak
/// sampler, and the settle-time tree is captured too; the reported `tree` is
/// whichever of the two has the higher `tree_rss_kib`, so a `node` child that
/// exits before settle is still attributed. Used by `skill-run`.
pub async fn measure_with_tree<F, Fut>(
    scenario: &'static str,
    workload_units: usize,
    turns: Option<usize>,
    workload: F,
) -> Result<ProfileResult>
where
    F: FnOnce(Recorder) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    measure_impl(scenario, workload_units, turns, true, workload).await
}

async fn measure_impl<F, Fut>(
    scenario: &'static str,
    workload_units: usize,
    turns: Option<usize>,
    sample_tree: bool,
    workload: F,
) -> Result<ProfileResult>
where
    F: FnOnce(Recorder) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    tokio::time::sleep(Duration::from_millis(250)).await;
    let baseline = proc_metrics::sample_self()?;
    if let Some(seconds) = std::env::var("OPENHUMAN_PROFILE_HOLD_BEFORE_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
    {
        eprintln!(
            "[library-profile] pid={} holding at baseline for {seconds}s",
            std::process::id()
        );
        tokio::time::sleep(Duration::from_secs(seconds)).await;
    }
    let sampler = PeakSampler::start(baseline.rss_kib);
    let tree_sampler = if sample_tree {
        Some(TreePeakSampler::start())
    } else {
        None
    };
    let started = Instant::now();
    let recorder = Recorder::new(baseline.rss_kib, started);
    eprintln!("[library-profile] scenario={scenario} workload starting");
    workload(recorder.clone()).await?;
    let duration_ms = started.elapsed().as_millis();
    eprintln!("[library-profile] scenario={scenario} workload done duration_ms={duration_ms}");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let settled = proc_metrics::sample_self()?;
    let peak_rss_kib = sampler.stop().await.max(settled.rss_kib);

    // Fold the process-tree samples (peak-during-workload + settle-time) into a
    // single report: whichever has the higher tree RSS wins, so a short-lived
    // interpreter child that already exited by settle is still attributed.
    let tree = if let Some(tree_sampler) = tree_sampler {
        let peak_tree = tree_sampler.stop().await;
        let settle_tree = proc_metrics::sample_tree().ok();
        let best = match (peak_tree, settle_tree) {
            (Some(peak), Some(settle)) => {
                if peak.tree_rss_kib >= settle.tree_rss_kib {
                    Some(peak)
                } else {
                    Some(settle)
                }
            }
            (peak, settle) => peak.or(settle),
        };
        if let Some(best) = best.as_ref() {
            eprintln!(
                "[library-profile] scenario={scenario} tree_rss_kib={} child_count={}",
                best.tree_rss_kib,
                best.children.len()
            );
        }
        best.map(|sample| TreeReport::from_sample(&sample))
    } else {
        None
    };
    let checkpoints = recorder.take();
    let checkpoints = if checkpoints.is_empty() {
        None
    } else {
        Some(checkpoints)
    };
    Ok(ProfileResult {
        schema_version: 2,
        scenario,
        workload_units,
        duration_ms,
        baseline,
        settled,
        peak_rss_kib,
        retained_delta_kib: settled.rss_kib as i64 - baseline.rss_kib as i64,
        peak_delta_kib: peak_rss_kib.saturating_sub(baseline.rss_kib),
        turns,
        agents: None,
        agents_built: None,
        marginal_rss_kib_per_agent: None,
        idle_cpu_ms: None,
        turn_latency_ms: None,
        budget: None,
        checkpoints,
        subagents: None,
        tree,
        dhat: None,
    })
}

//! `SubconsciousInstance` — the generic, world-agnostic subconscious runner.
//!
//! One instance ticks one [`SubconsciousProfile`] (`memory`, `tinyplace`, …).
//! The tick body is a [`tinyagents` `CompiledGraph`](tinyagents::graph) — the
//! same durable-workflow runtime the orchestration wake path runs on:
//!
//! ```text
//! START ─► observe ─┬─(quiet)────────────────────────────► commit ─► done ─► END
//!                   └─(changed)─► prepare ─► reflect ─┬─(ok)──► commit ─► done ─► END
//!                                                     └─(err)────────────► done ─► END
//! ```
//!
//! The node handlers delegate 1:1 to the profile — the profile trait **is** the
//! injected runtime (mirroring `OrchestrationRuntime`). Everything world-agnostic
//! lives here, **once**, so every world gets it for free: the tick lock + 5s
//! acquisition skip, the generation/supersede counter, the `TICK_TIMEOUT` wall
//! clock, the provider gate + per-instance rate-cap halt (TAURI-RUST-HXF), the
//! tool-capability classifier (TAURI-RUST-ADC), only-advance-baselines-on-success,
//! and the quiet-tick short-circuit.
//!
//! Deviation from the plan sketch: the graph is rebuilt per tick (capturing the
//! freshly-loaded `Config` + the profile in the node closures) rather than once
//! at construction, so each tick reflects the live provider config. This is the
//! established `orchestration::graph::build::run_orchestration_graph` pattern and
//! does not affect checkpoint resume, which is keyed by thread id on disk.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tinyagents::graph::{
    Checkpointer, ClosureStateReducer, Command, CompiledGraph, GraphBuilder, NodeContext,
    NodeResult, SqliteCheckpointer,
};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use super::profile::{Observation, Reflection, SubconsciousProfile};
use super::provider::{
    evaluate_rate_cap_halt, is_permanent_rate_cap_error, is_tool_capability_error,
    subconscious_provider_signature, subconscious_provider_unavailable_reason, RateCapHaltDecision,
    RATE_CAP_HALT_REASON, TOOL_UNSUPPORTED_REASON,
};
use super::types::{SubconsciousStatus, TickResult};
use crate::openhuman::config::Config;
use crate::openhuman::tinyagents::observability::GraphTracingSink;

/// Hard timeout for a single subconscious tick.
const TICK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// The state threaded through one tick graph. Serde so a killed tick can resume
/// from the last checkpoint boundary rather than losing its window.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SubconsciousState {
    /// Stable per-tick id (`<unix_secs>`), scoping the checkpoint thread.
    tick_id: String,
    /// The generation this tick was born with — compared against the shared
    /// counter in the `commit` node so a superseded tick never advances state.
    generation: u64,
    observation: Observation,
    prepared_context: String,
    reflection: Option<Reflection>,
    reflect_error: Option<String>,
    committed: bool,
}

/// Reducer update emitted by a subconscious node (one per node result).
enum SubconsciousUpdate {
    Observed(Observation),
    Prepared(String),
    Reflected(Reflection),
    ReflectFailed(String),
    Committed,
    Noop,
}

/// The instance-local, in-memory counters and circuit-breaker state. Guarded by
/// a small mutex the `status()` path can take without ever touching `tick_lock`
/// (invariant: status never blocks on a running tick).
struct InstanceState {
    last_tick_at: f64,
    total_ticks: u64,
    consecutive_failures: u64,
    provider_unavailable_reason: Option<String>,
    /// Signature (`"<id>|<provider-sig>"`) of a permanently-rejecting provider
    /// (413/TPM). While set and still matching the live config, ticks skip the
    /// reflect run. Cleared when the config's signature changes. In-memory only.
    /// TAURI-RUST-HXF.
    rate_cap_halt_signature: Option<String>,
}

impl InstanceState {
    /// Pre-tick gate: consult the rate-cap halt against the live signature.
    /// Returns `true` when the tick must skip because a halt is active for the
    /// still-current config. A halt whose signature no longer matches is cleared
    /// here and the tick proceeds. Counts a skipped tick. TAURI-RUST-HXF.
    fn should_skip_for_rate_cap_halt(&mut self, signature: &str, log_prefix: &str) -> bool {
        match evaluate_rate_cap_halt(self.rate_cap_halt_signature.as_deref(), signature) {
            RateCapHaltDecision::Skip => {
                info!(
                    "{log_prefix} halted — provider keeps hitting a permanent per-minute token cap \
                     (413/TPM); skipping tick until the model/tier changes (TAURI-RUST-HXF)"
                );
                self.total_ticks += 1;
                true
            }
            RateCapHaltDecision::Resume => {
                info!("{log_prefix} provider config changed — clearing rate-cap halt, resuming");
                self.rate_cap_halt_signature = None;
                if self.provider_unavailable_reason.as_deref() == Some(RATE_CAP_HALT_REASON) {
                    self.provider_unavailable_reason = None;
                }
                false
            }
            RateCapHaltDecision::Proceed => false,
        }
    }

    /// Arm the rate-cap halt after a tick failed with a permanent per-minute
    /// token-cap rejection. TAURI-RUST-HXF.
    fn arm_rate_cap_halt(&mut self, signature: &str, log_prefix: &str) {
        info!(
            "{log_prefix} provider rejected the tick with a permanent per-minute token cap \
             (413/TPM) — halting until the model/tier changes (TAURI-RUST-HXF)"
        );
        self.rate_cap_halt_signature = Some(signature.to_string());
        self.provider_unavailable_reason = Some(RATE_CAP_HALT_REASON.to_string());
    }
}

pub struct SubconsciousInstance {
    profile: Arc<dyn SubconsciousProfile>,
    workspace_dir: PathBuf,
    enabled: bool,
    interval_minutes: u32,
    /// Display label for `status().mode` — the memory instance maps its
    /// `SubconsciousMode`; worlds with no mode concept pass a fixed label.
    mode_label: String,
    state: Mutex<InstanceState>,
    /// Shared with the graph's `commit` node so a superseded tick self-skips its
    /// commit between supersteps, not just at the end.
    tick_generation: Arc<AtomicU64>,
    tick_lock: Mutex<()>,
}

impl SubconsciousInstance {
    /// Construct an instance around `profile`, resuming its persisted
    /// `last_tick_at` from disk. `enabled`, `interval_minutes` and `mode_label`
    /// are supplied by the caller (factory / bootstrap) since they derive from
    /// world-specific config the profile already read.
    pub fn new(
        profile: Arc<dyn SubconsciousProfile>,
        workspace_dir: PathBuf,
        enabled: bool,
        interval_minutes: u32,
        mode_label: impl Into<String>,
    ) -> Self {
        let instance_id = profile.id();
        let last_tick_at = match super::store::with_connection(&workspace_dir, |conn| {
            super::store::get_last_tick_at(conn, instance_id)
        }) {
            Ok(v) => {
                if v > 0.0 {
                    info!("[subconscious:{instance_id}] resumed last_tick_at={v} from disk");
                }
                v
            }
            Err(e) => {
                warn!("[subconscious:{instance_id}] last_tick_at load failed, using 0.0: {e}");
                0.0
            }
        };

        Self {
            profile,
            workspace_dir,
            enabled,
            interval_minutes,
            mode_label: mode_label.into(),
            state: Mutex::new(InstanceState {
                last_tick_at,
                total_ticks: 0,
                consecutive_failures: 0,
                provider_unavailable_reason: None,
                rate_cap_halt_signature: None,
            }),
            tick_generation: Arc::new(AtomicU64::new(0)),
            tick_lock: Mutex::new(()),
        }
    }

    fn log_prefix(&self) -> String {
        format!("[subconscious:{}]", self.profile.id())
    }

    /// Run one tick: acquire the per-instance tick lock (skip if another tick
    /// holds it after 5s), then run the tick under the hard `TICK_TIMEOUT`.
    pub async fn tick(&self) -> Result<TickResult> {
        let prefix = self.log_prefix();
        let _tick_guard =
            match tokio::time::timeout(std::time::Duration::from_secs(5), self.tick_lock.lock())
                .await
            {
                Ok(guard) => guard,
                Err(_) => {
                    warn!("{prefix} tick skipped — another tick is still running");
                    return Ok(TickResult {
                        tick_at: now_secs(),
                        duration_ms: 0,
                        response_chars: 0,
                    });
                }
            };

        match tokio::time::timeout(TICK_TIMEOUT, self.tick_inner()).await {
            Ok(result) => result,
            Err(_) => {
                warn!("{prefix} tick timed out after {}s", TICK_TIMEOUT.as_secs());
                let mut state = self.state.lock().await;
                state.consecutive_failures += 1;
                state.total_ticks += 1;
                Ok(TickResult {
                    tick_at: now_secs(),
                    duration_ms: TICK_TIMEOUT.as_millis() as u64,
                    response_chars: 0,
                })
            }
        }
    }

    async fn tick_inner(&self) -> Result<TickResult> {
        let config = match Config::load_or_init().await {
            Ok(c) => c,
            Err(e) => {
                warn!("{} config load failed: {e}", self.log_prefix());
                let mut state = self.state.lock().await;
                state.provider_unavailable_reason = Some(format!("Config unavailable: {e}"));
                state.consecutive_failures += 1;
                state.total_ticks += 1;
                return Ok(TickResult {
                    tick_at: now_secs(),
                    duration_ms: 0,
                    response_chars: 0,
                });
            }
        };
        self.run_tick(config).await
    }

    /// The tick body given a loaded config. Split out so tests can drive it with
    /// a crafted `Config` without hitting disk (`Config::load_or_init`).
    async fn run_tick(&self, config: Config) -> Result<TickResult> {
        let prefix = self.log_prefix();
        let started = std::time::Instant::now();
        let tick_at = now_secs();
        let my_generation = self.tick_generation.fetch_add(1, Ordering::SeqCst) + 1;

        // ── Provider gate + rate-cap halt (per-instance signature) ───────────
        let provider_signature = format!(
            "{}|{}",
            self.profile.id(),
            subconscious_provider_signature(&config)
        );
        if self
            .state
            .lock()
            .await
            .should_skip_for_rate_cap_halt(&provider_signature, &prefix)
        {
            return Ok(self.quiet_result(tick_at, started));
        }
        if let Some(reason) = subconscious_provider_unavailable_reason(&config) {
            info!("{prefix} provider unavailable, skipping tick: {reason}");
            let mut state = self.state.lock().await;
            state.provider_unavailable_reason = Some(reason);
            state.consecutive_failures += 1;
            state.total_ticks += 1;
            return Ok(self.quiet_result(tick_at, started));
        }
        {
            let mut state = self.state.lock().await;
            state.provider_unavailable_reason = None;
        }

        // ── Run the tick graph (observe → [prepare → reflect] → commit) ──────
        let seed = SubconsciousState {
            tick_id: format!("{}", tick_at as u64),
            generation: my_generation,
            ..Default::default()
        };
        let final_state = match self.run_graph(&config, seed).await {
            Ok(s) => s,
            Err(e) => {
                warn!("{prefix} tick graph run failed: {e}");
                let mut state = self.state.lock().await;
                state.consecutive_failures += 1;
                state.total_ticks += 1;
                return Ok(self.quiet_result(tick_at, started));
            }
        };

        // ── Superseded-generation discard ────────────────────────────────────
        if self.tick_generation.load(Ordering::SeqCst) != my_generation {
            info!("{prefix} tick superseded by newer tick, discarding");
            let mut state = self.state.lock().await;
            state.total_ticks += 1;
            return Ok(self.quiet_result(tick_at, started));
        }

        // ── Per-instance state/status update ─────────────────────────────────
        let response_chars = match &final_state.reflection {
            Some(Reflection::Acted { response_chars }) => *response_chars,
            _ => 0,
        };
        let mut state = self.state.lock().await;
        state.total_ticks += 1;
        if let Some(err) = &final_state.reflect_error {
            state.consecutive_failures += 1;
            if is_tool_capability_error(err) {
                info!("{prefix} configured chat model has no tool-use endpoint (TAURI-RUST-ADC)");
                state.provider_unavailable_reason = Some(TOOL_UNSUPPORTED_REASON.to_string());
            } else if is_permanent_rate_cap_error(err) {
                state.arm_rate_cap_halt(&provider_signature, &prefix);
            }
        } else {
            // Quiet tick or successful reflect — both advance.
            state.consecutive_failures = 0;
            state.last_tick_at = tick_at;
            persist_last_tick_at(&self.workspace_dir, self.profile.id(), tick_at);
        }

        Ok(TickResult {
            tick_at,
            duration_ms: started.elapsed().as_millis() as u64,
            response_chars,
        })
    }

    fn quiet_result(&self, tick_at: f64, started: std::time::Instant) -> TickResult {
        TickResult {
            tick_at,
            duration_ms: started.elapsed().as_millis() as u64,
            response_chars: 0,
        }
    }

    /// Build and run the per-tick graph, checkpointing at every super-step
    /// boundary under thread `subconscious:<instance>:<tick_id>`.
    async fn run_graph(
        &self,
        config: &Config,
        seed: SubconsciousState,
    ) -> anyhow::Result<SubconsciousState> {
        let graph = self.build_graph(config)?;
        let thread_id = format!("subconscious:{}:{}", self.profile.id(), seed.tick_id);
        let checkpoint_db = self
            .workspace_dir
            .join("subconscious")
            .join("graph_checkpoints.db");
        let checkpointer = Arc::new(
            SqliteCheckpointer::<SubconsciousState>::open(&checkpoint_db)
                .map_err(|e| anyhow::anyhow!("open subconscious checkpoint store: {e}"))?,
        );
        // Keep a handle for post-run GC. Each tick uses a unique thread id, so a
        // *completed* tick's checkpoints are dead weight — ticks run every N
        // minutes forever, so without pruning `graph_checkpoints.db` grows
        // unboundedly (phase 6: adopt the checkpointer's existing retention
        // primitive rather than adding one upstream).
        let gc = Arc::clone(&checkpointer);
        let graph = graph
            .with_checkpointer(checkpointer)
            .with_event_sink(Arc::new(GraphTracingSink::new(thread_id.clone())));
        let exec = graph
            .run_with_thread(thread_id.clone(), seed)
            .await
            .map_err(|e| anyhow::anyhow!("subconscious graph run failed: {e}"))?;
        // The run returned; resume value is spent. Drop this tick's thread so the
        // checkpoint db stays bounded. Best-effort — a GC failure is not a tick
        // failure.
        if let Err(e) = gc.delete_thread(&thread_id).await {
            debug!(
                "{} checkpoint GC failed for {thread_id}: {e}",
                self.log_prefix()
            );
        }
        Ok(exec.state)
    }

    fn build_graph(
        &self,
        config: &Config,
    ) -> anyhow::Result<CompiledGraph<SubconsciousState, SubconsciousUpdate>> {
        let profile = self.profile.clone();
        let config = Arc::new(config.clone());
        let generation = self.tick_generation.clone();
        let log_prefix = self.log_prefix();

        let mut builder = GraphBuilder::<SubconsciousState, SubconsciousUpdate>::new().set_reducer(
            ClosureStateReducer::new(|mut s: SubconsciousState, u: SubconsciousUpdate| {
                match u {
                    SubconsciousUpdate::Observed(obs) => s.observation = obs,
                    SubconsciousUpdate::Prepared(ctx) => s.prepared_context = ctx,
                    SubconsciousUpdate::Reflected(r) => s.reflection = Some(r),
                    SubconsciousUpdate::ReflectFailed(e) => s.reflect_error = Some(e),
                    SubconsciousUpdate::Committed => s.committed = true,
                    SubconsciousUpdate::Noop => {}
                }
                Ok(s)
            }),
        );

        // `observe`: what changed in this world? Routes quiet → commit,
        // changed → prepare.
        {
            let profile = profile.clone();
            let config = config.clone();
            let prefix = log_prefix.clone();
            builder = builder.add_node("observe", move |_s: SubconsciousState, _c: NodeContext| {
                let profile = profile.clone();
                let config = config.clone();
                let prefix = prefix.clone();
                async move {
                    let obs = profile.observe(&config).await;
                    let route = if obs.has_changes { "prepare" } else { "commit" };
                    debug!(
                        "{prefix} node.observe has_changes={} external={} route={route}",
                        obs.has_changes, obs.has_external_content,
                    );
                    Ok(NodeResult::Command(
                        Command::default()
                            .with_update(SubconsciousUpdate::Observed(obs))
                            .with_goto([route]),
                    ))
                }
            });
        }

        // `prepare`: grounding context for the reflection turn (changed only).
        {
            let profile = profile.clone();
            let config = config.clone();
            builder = builder.add_node("prepare", move |s: SubconsciousState, _c: NodeContext| {
                let profile = profile.clone();
                let config = config.clone();
                async move {
                    let ctx = profile.prepare_context(&config, &s.observation).await;
                    Ok(NodeResult::Update(SubconsciousUpdate::Prepared(ctx)))
                }
            });
        }

        // `reflect`: the reflection turn. Routes ok → commit, err → done (a
        // failed reflect must not advance the baseline).
        {
            let profile = profile.clone();
            let config = config.clone();
            let prefix = log_prefix.clone();
            builder = builder.add_node("reflect", move |s: SubconsciousState, _c: NodeContext| {
                let profile = profile.clone();
                let config = config.clone();
                let prefix = prefix.clone();
                async move {
                    let origin = profile.origin(&s.observation);
                    debug!("{prefix} node.reflect origin={origin:?}");
                    match profile
                        .reflect(&config, &s.observation, &s.prepared_context)
                        .await
                    {
                        Ok(reflection) => Ok(NodeResult::Command(
                            Command::default()
                                .with_update(SubconsciousUpdate::Reflected(reflection))
                                .with_goto(["commit"]),
                        )),
                        Err(e) => {
                            warn!("{prefix} node.reflect failed: {e}");
                            Ok(NodeResult::Command(
                                Command::default()
                                    .with_update(SubconsciousUpdate::ReflectFailed(e))
                                    .with_goto(["done"]),
                            ))
                        }
                    }
                }
            });
        }

        // `commit`: advance the world baseline/cursor — but only if this tick
        // has not been superseded by a newer one (checked against the shared
        // generation counter, the graph twin of an external cancel token).
        {
            let profile = profile.clone();
            let config = config.clone();
            let prefix = log_prefix.clone();
            builder = builder.add_node("commit", move |s: SubconsciousState, _c: NodeContext| {
                let profile = profile.clone();
                let config = config.clone();
                let generation = generation.clone();
                let prefix = prefix.clone();
                async move {
                    if generation.load(Ordering::SeqCst) != s.generation {
                        debug!("{prefix} node.commit skipped — tick superseded");
                        return Ok(NodeResult::Update(SubconsciousUpdate::Noop));
                    }
                    profile.commit(&config, &s.observation).await;
                    debug!("{prefix} node.commit advanced baseline");
                    Ok(NodeResult::Update(SubconsciousUpdate::Committed))
                }
            });
        }

        let graph = builder
            .add_node(
                "done",
                |_s: SubconsciousState, _c: NodeContext| async move {
                    Ok(NodeResult::Update(SubconsciousUpdate::Noop))
                },
            )
            .add_edge("prepare", "reflect")
            .add_edge("commit", "done")
            .set_entry("observe")
            .mark_command_routing("observe")
            .mark_command_routing("reflect")
            .set_finish("done")
            .compile()
            .map_err(|e| anyhow::anyhow!("subconscious graph compile failed: {e}"))?;
        Ok(graph)
    }

    /// Stable world id (`"memory"` | `"tinyplace"`) — for logging + fan-out.
    pub fn id(&self) -> &'static str {
        self.profile.id()
    }

    /// Whether this instance's cadence has elapsed since its last successful
    /// tick (a never-ticked instance is always due). Pure read of the small
    /// `state` mutex — never touches `tick_lock`, so the heartbeat fan-out can
    /// poll it without ever blocking on a running tick.
    pub async fn is_due(&self, now: f64) -> bool {
        let last = self.state.lock().await.last_tick_at;
        if last <= 0.0 {
            return true;
        }
        now - last >= f64::from(self.interval_minutes) * 60.0
    }

    pub async fn status(&self) -> SubconsciousStatus {
        let state = self.state.lock().await;
        SubconsciousStatus {
            instance: self.profile.id().to_string(),
            enabled: self.enabled,
            mode: self.mode_label.clone(),
            provider_available: state.provider_unavailable_reason.is_none(),
            provider_unavailable_reason: state.provider_unavailable_reason.clone(),
            interval_minutes: self.interval_minutes,
            last_tick_at: (state.last_tick_at > 0.0).then_some(state.last_tick_at),
            total_ticks: state.total_ticks,
            consecutive_failures: state.consecutive_failures,
        }
    }
}

#[cfg(test)]
impl SubconsciousInstance {
    /// Drive one tick with a caller-supplied `Config`, skipping the tick lock,
    /// timeout, and `Config::load_or_init` disk read. Test seam only.
    pub(crate) async fn run_tick_for_test(&self, config: Config) -> Result<TickResult> {
        self.run_tick(config).await
    }

    /// A clone of the shared generation counter so a test profile can simulate a
    /// newer tick superseding the one in flight.
    pub(crate) fn generation_handle(&self) -> Arc<AtomicU64> {
        self.tick_generation.clone()
    }

    pub(crate) async fn snapshot_failures(&self) -> u64 {
        self.state.lock().await.consecutive_failures
    }
}

fn persist_last_tick_at(workspace_dir: &std::path::Path, instance: &str, tick_at: f64) {
    if let Err(e) = super::store::with_connection(workspace_dir, |conn| {
        super::store::set_last_tick_at(conn, instance, tick_at)
    }) {
        warn!("[subconscious:{instance}] failed to persist last_tick_at={tick_at}: {e}");
    }
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
#[path = "instance_tests.rs"]
mod tests;

# Phase 1 — `SubconsciousProfile` trait + generic instance runner

Goal: introduce the abstraction without moving any behavior yet. At the end of
this phase the existing `SubconsciousEngine` still works exactly as today; the
new types compile alongside it and are exercised only by unit tests.

## 1.1 `profile.rs` — the world contract

```rust
/// One "world" a subconscious can be instantiated over.
#[async_trait::async_trait]
pub trait SubconsciousProfile: Send + Sync {
    /// Stable instance id — store-key namespace, log prefix, RPC name.
    fn id(&self) -> &'static str; // "memory" | "tinyplace"

    /// Tick cadence for this world (heartbeat multiplies its base interval).
    fn cadence(&self, config: &Config) -> std::time::Duration;

    /// Stage 1: what changed in this world since my baseline?
    /// Errors and first-ever ticks surface as an empty observation
    /// (`has_changes == false`) so the runner's quiet-path handles them.
    async fn observe(&self, config: &Config) -> Observation;

    /// Stage 2 (optional): grounding context for the reflection turn.
    /// Default impl returns "" (the tinyplace profile skips this stage).
    async fn prepare_context(&self, _config: &Config, _obs: &Observation) -> String {
        String::new()
    }

    /// Stage 3: the reflection turn. Runs only when `obs.has_changes`.
    async fn reflect(
        &self,
        config: &Config,
        obs: &Observation,
        prepared_context: &str,
    ) -> Result<Reflection, String>;

    /// Advance this world's baseline/cursor. Called by the runner:
    /// - after a quiet tick (refresh the baseline), and
    /// - after a successful reflect (never after a failed or superseded one).
    async fn commit(&self, config: &Config, obs: &Observation);

    /// Turn-origin taint for this observation (memory: tainted iff the diff
    /// carried external content; tinyplace: always tainted).
    fn origin(&self, obs: &Observation) -> TrustedAutomationSource;
}

pub struct Observation {
    /// Rendered world diff handed to `reflect` (empty when quiet).
    pub rendered: String,
    pub has_changes: bool,
    /// Whether the change window contains third-party content (taint input).
    pub has_external_content: bool,
    /// Opaque token `commit` uses to advance exactly the observed window
    /// (memory: none needed — it re-checkpoints; tinyplace: newest reviewed
    /// compressed-row created_at for the review cursor).
    pub commit_token: Option<String>,
}

pub enum Reflection {
    /// The decision agent acted through tools (memory profile).
    Acted { response_chars: usize },
    /// A steering directive was emitted (tinyplace profile).
    Steered { directive_id: i64 },
    /// The model looked and correctly chose to do nothing.
    Idle,
}
```

Design notes:

- `observe` is **infallible by signature**: today `tick_inner` already treats a
  diff error as "quiet tick, refresh baseline"; encoding that in the type keeps
  the runner branch-free. The profile logs its own errors.
- `reflect` returns `Err(String)` (matching today's `run_agent` error channel)
  so the runner can keep the existing error classifiers — the string is fed to
  `is_tool_capability_error` / `is_permanent_rate_cap_error` unchanged.
- `commit_token` exists because the tinyplace world advances a *cursor over
  reviewed rows* (must not skip rows that arrived mid-tick), while the memory
  world re-checkpoints the whole world. Both fit one optional string.

## 1.2 `engine.rs` — `SubconsciousInstance` (generic runner **as a tinyagents graph**)

Rename the struct conceptually: `SubconsciousEngine` → `SubconsciousInstance`
(keep a `pub type SubconsciousEngine = SubconsciousInstance;` alias until
phase 4 updates the callers). The instance owns exactly what the engine owns
today, minus every memory-specific line:

```rust
pub struct SubconsciousInstance {
    profile: Arc<dyn SubconsciousProfile>,
    graph: Arc<CompiledGraph<SubconsciousState>>,   // built once at construction
    workspace_dir: PathBuf,
    mode: SubconsciousMode,
    interval_minutes: u32,
    enabled: bool,
    state: Mutex<EngineState>,        // unchanged fields
    tick_generation: AtomicU64,
    tick_lock: Mutex<()>,
}
```

The tick body is **not** hand-rolled control flow: it is one `tinyagents`
[`CompiledGraph`] built with `GraphBuilder`, exactly the pattern
`orchestration/graph/build.rs` established (nodes call an injected
`dyn` runtime; the graph owns routing/termination; a stub runtime makes the
mechanics unit-testable). Topology:

```text
START ─► observe ─┬─(quiet)────────────────────────────► commit ─► END
                  └─(changed)─► prepare_context ─► reflect ─► commit ─► END
```

- `SubconsciousState` (serde, like `OrchestrationState`): the `Observation`,
  `prepared_context`, `Reflection` outcome, tick metadata (`tick_id`,
  generation). Channels are plain `LastValue`s — no message reducers needed.
- The node handlers delegate 1:1 to the `SubconsciousProfile` methods — the
  profile trait **is** the injected runtime (mirroring
  `OrchestrationRuntime`); no second abstraction.
- Conditional edge out of `observe` on `has_changes` (command-routing, same
  mechanism as the `frontend` router).
- Checkpointing: `SqliteCheckpointer<SubconsciousState>` at
  `<workspace>/subconscious/graph_checkpoints.db`, thread id
  `subconscious:<instance>:<tick_id>`. A tick killed mid-reflect resumes (or
  is superseded cleanly) instead of losing the window — same durability story
  the wake graph already has. Baseline advancement stays in `commit`, so a
  resumed tick can never double-advance.
- Observability for free: graph stream events / status snapshots
  (`tinyagents::graph::stream`/`status`) give per-node timing without bespoke
  logging (keep the `[subconscious]` log lines anyway per repo logging rules).

What deliberately stays *outside* the graph, in `SubconsciousInstance`:
the cadence loop trigger (heartbeat), the tick lock + 5s acquisition skip,
the generation counter (supersede), `TICK_TIMEOUT`, provider gate + rate-cap
halt, and status. Rationale: these are scheduler/circuit-breaker concerns
identical to what `ops::invoke_with_runtime` keeps outside the wake graph —
see phase 6 for the tinyagents-side gaps that would let some of them move
into the runtime later.

The pieces that stay verbatim in the runner:

- 5s tick-lock acquisition timeout + skip log
- `TICK_TIMEOUT` wall clock (30 min) — consider making it a profile constant
  later; not in this phase
- config-load failure path
- provider gate (`subconscious_provider_unavailable_reason`) and the rate-cap
  halt (`should_skip_for_rate_cap_halt` / `arm_rate_cap_halt`) — the halt
  signature gains the instance id prefix (`"memory|cloud"`) so one world's
  halt never silences another
- tool-capability / rate-cap error classification on `reflect` failure
- superseded-generation discard
- `status()` — gains `instance: String` (the profile id)

## 1.3 `store.rs` — instance-namespaced KV + migration

Today: `subconscious_state` (REAL KV: `last_tick_at`) and
`subconscious_state_text` (TEXT KV: `baseline_checkpoint_id`), one row each.

Change the accessors to take the instance id and prefix the key
(`"memory:last_tick_at"`, `"tinyplace:last_tick_at"`, …). One-time migration in
the DDL block of `with_connection` (idempotent, like all our DDL):

```sql
UPDATE subconscious_state      SET key = 'memory:' || key
  WHERE key IN ('last_tick_at') AND NOT EXISTS (SELECT 1 FROM subconscious_state WHERE key = 'memory:last_tick_at');
UPDATE subconscious_state_text SET key = 'memory:' || key
  WHERE key IN ('baseline_checkpoint_id') AND NOT EXISTS (SELECT 1 FROM subconscious_state_text WHERE key = 'memory:baseline_checkpoint_id');
```

(Exact guard shape to be settled in implementation — requirement is: running
old→new→old… never loses `last_tick_at`, and the migration is a no-op on a
fresh DB. The tinyplace profile's *review cursor* stays where it lives today,
in the orchestration store — the profile owns it via `commit`.)

## 1.4 Tests (this phase)

- A `FakeProfile` (scripted observations/reflections, call recorder) driving
  `SubconsciousInstance` directly:
  - quiet observation → no `reflect`, `commit` called, `last_tick_at` advanced
  - failing `reflect` → no `commit`, `consecutive_failures` bumped, baseline untouched
  - rate-cap error string → halt armed under `"<id>|<provider-sig>"`; second
    tick skips; signature change resumes
  - superseded generation → result discarded, no `commit`
- Store: namespaced get/set round-trip; legacy-key migration (seed old keys,
  open, assert `memory:`-keys carry the values).

Existing `engine_tests.rs` keeps passing untouched (the old engine still
exists in this phase).

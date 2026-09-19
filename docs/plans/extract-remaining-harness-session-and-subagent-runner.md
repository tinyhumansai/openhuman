# Extract the remaining session and sub-agent runtime from OpenHuman

## End state and rules

This extraction deletes `crates/openhuman-core/src/agent/harness/session/` and
`crates/openhuman-core/src/agent/harness/subagent_runner/`. It must not replace
them with aliases, forwarding modules, compatibility re-exports, wrappers, or
dual live paths. Every migrated caller imports its final owner directly.

Final ownership:

| Owner | Responsibility |
| --- | --- |
| `tinyagents-harness` | generic `RunQueue<T>` and `ResultHandoffCache`, model/tool loop and cancellation mechanics |
| `tinyagents-session` | transcript storage/lookup/append/compaction and layout migration |
| new `tinyagents-runtime` | generic stateful session builder/driver, history, prefix/tool snapshots, transcript deltas and partial persistence |
| `tinyagents-orchestration::subagent` | generic plan/prepare/execute/pause/persist sub-agent lifecycle |
| `agent/session_host` | OpenHuman prompt, memory/experience, security, progress/BUS, finalization, tool policy and factories |
| `agent/subagent_host` and orchestration host adapters | OpenHuman definition/tier/model/tool/security/memory/artifact/progress/mirroring policy and RPC/tool DTOs |

No extraction may weaken these invariants: terminal lifecycle events are unique;
cancellation is propagated and has one truthful terminal result; nested usage
rolls up exactly once even after outer error/cancellation; task-local values
are propagated through explicit run contexts; transcript wire/storage paths
and legacy behavior remain byte-compatible; provider-visible, dispatchable and
delegated tool surfaces use the same fail-closed allowlist; and graph bindings
remain live execution data, never persisted state.

## Dependency direction

```text
tinytools
   ^
tinyagents-harness <--- tinyagents-session
   ^                       ^
   |                       |
tinyagents-runtime --------+
   ^
tinyagents-orchestration::{subagent,teams,workflow}
   ^
OpenHuman agent/{session_host,subagent_host,orchestration/**}
```

Register `tinyagents-runtime` in `vendor/tinyagents/Cargo.toml`. It depends on
`tinyagents-harness`, `tinyagents-session`, and `tinytools` (and neutral
implementation dependencies such as `anyhow`, `async-trait`, `serde`, and
`tokio`), never OpenHuman, product configuration, credentials, RPC, prompts,
or BUS. `tinyagents-orchestration` may depend on graph/harness/session/runtime
and tinytools; none of those crates may depend back on it. The session crate
may receive a generic subagent persistence adapter only if its public API names
only session-owned values. Otherwise `SubagentPersistence` is implemented by
`agent/subagent_host`, which preserves dependency direction.

OpenHuman retains `AgentDefinition`, tiers, model/provider choice, tool and
security policy, OpenHuman memory/artifact/progress/BUS behavior, worker
mirroring, and request-mode/DTO mapping. `QueueMode` and its queued message DTO
stay host-side even when the FIFO itself is direct upstream use.

## Required public APIs

### New `tinyagents-runtime`

Create `vendor/tinyagents/crates/tinyagents-runtime/{Cargo.toml,src/lib.rs}`
with focused `builder`, `session`, `driver`, `history`, `prefix`, `tools`,
`transcript_delta`, `persistence`, `hooks`, `types`, `error`, and `test`
modules. The public contract must be host-neutral and object-safe:

```rust
pub struct TurnOptions {
    pub request_id: Option<String>,
    pub thread_id: Option<String>,
    pub stream: bool,
    pub resume: ResumeMode,
    pub cancellation: tinyagents_harness::CancellationToken,
}

pub trait TranscriptCodec: Send + Sync {
    fn decode_history(
        &self,
        transcript: &tinyagents_session::transcript::SessionTranscript,
    ) -> Result<Vec<tinyinference_llm::message::Message>, RuntimeError>;
    fn encode_delta(
        &self,
        previous: &[tinyinference_llm::message::Message],
        next: &[tinyinference_llm::message::Message],
        options: &TurnOptions,
    ) -> Result<tinyagents_session::transcript::TranscriptTurnDelta, RuntimeError>;
}

#[async_trait::async_trait]
pub trait SessionHooks: Send + Sync {
    async fn before_turn(&self, request: &mut SessionTurnRequest) -> Result<(), RuntimeError>;
    async fn after_turn(&self, outcome: &SessionTurnOutcome) -> Result<(), RuntimeError>;
    async fn on_terminal(&self, terminal: &SessionTerminal) -> Result<(), RuntimeError>;
}

pub struct SessionBuilder { /* private fields */ }
pub struct Session { /* private history, prefix, tool snapshot and driver */ }

impl SessionBuilder {
    pub fn new(harness: Arc<tinyagents_harness::runtime::AgentHarness<...>>) -> Self;
    pub fn transcript_locator(self, Arc<dyn TranscriptLocator>) -> Self;
    pub fn codec(self, Arc<dyn TranscriptCodec>) -> Self;
    pub fn hooks(self, Arc<dyn SessionHooks>) -> Self;
    pub fn tool_snapshot(self, ToolSnapshot) -> Self;
    pub fn build(self) -> Result<Session, RuntimeError>;
}
impl Session {
    pub async fn turn(&mut self, SessionTurnRequest, TurnOptions)
        -> Result<SessionTurnOutcome, RuntimeError>;
    pub async fn resume(&mut self, TurnOptions) -> Result<SessionResume, RuntimeError>;
    pub fn history(&self) -> &[tinyinference_llm::message::Message];
    pub fn prefix_snapshot(&self) -> &PrefixSnapshot;
    pub fn tool_snapshot(&self) -> &ToolSnapshot;
}
```

`SessionTurnRequest`, `SessionTurnOutcome`, `SessionTerminal`, `SessionResume`,
`PrefixSnapshot`, `ToolSnapshot`, and `RuntimeError` are runtime-owned neutral
types. Concrete TinyAgents messages are used internally; no OpenHuman type is
accepted. Both traits stay object-safe: no generic methods, `Self` returns, or
host-associated types. The runtime owns stateful history, stable prefix,
per-turn immutable tool snapshot, transcript delta/compaction and partial
persistence. It never authorizes a tool, composes a product prompt, loads
memory, chooses a model, or emits a product event.

### `tinyagents-session` migration API

Move `agent/harness/session/migration.rs` to
`vendor/tinyagents/crates/tinyagents-session/src/transcript/migration.rs` and
export:

```rust
pub struct TranscriptLayoutMigration { /* counts, warnings, already_done */ }
pub fn migrate_layout_if_needed(root: &Path) -> anyhow::Result<TranscriptLayoutMigration>;
```

Retain marker idempotency, collision/no-overwrite behavior, warnings,
DDMMYYYY JSONL flattening, YYYY_MM_DD Markdown directory migration and safe
pruning. The root is caller supplied and no OpenHuman configuration is named.

### `tinyagents-orchestration::subagent`

Create `vendor/tinyagents/crates/tinyagents-orchestration/src/subagent/{mod,
types,planner,executor,persistence,driver,test}.rs` and export:

```rust
pub struct SubagentRequest { pub task_id: String, pub parent_run: RunContext,
    pub input: String, pub thread_id: Option<String>, pub resume: Option<SubagentResume> }
pub struct PreparedSubagent { pub task_id: String, pub agent_key: String,
    pub input: Vec<Message>, pub tools: ToolSnapshot, pub run_context: RunContext }
pub struct SubagentExecution { pub prepared: PreparedSubagent, pub cancellation: CancellationToken }
pub struct SubagentOutcome { pub task_id: String, pub output: String, pub history: Vec<Message>,
    pub status: SubagentStatus, pub usage: UsageTotals, pub artifacts: Vec<ArtifactReference> }
pub enum SubagentStatus { Completed, AwaitingInput(SubagentPause), Incomplete(SubagentIncomplete), Cancelled }
pub enum SubagentError { Planning(String), Execution(String), Persistence(String), Cancelled }

#[async_trait::async_trait]
pub trait SubagentPlanner: Send + Sync {
    async fn prepare(&self, request: SubagentRequest) -> Result<PreparedSubagent, SubagentError>;
}
#[async_trait::async_trait]
pub trait SubagentExecutor: Send + Sync {
    async fn execute(&self, execution: SubagentExecution) -> Result<SubagentOutcome, SubagentError>;
}
#[async_trait::async_trait]
pub trait SubagentPersistence: Send + Sync {
    async fn load(&self, task_id: &str) -> Result<Option<SubagentResume>, SubagentError>;
    async fn save_pause(&self, pause: PersistedSubagentPause) -> Result<(), SubagentError>;
    async fn record_terminal(&self, outcome: &SubagentOutcome) -> Result<(), SubagentError>;
}
```

The orchestration driver only runs planner -> optional load -> executor -> one
persistence terminal action. All OpenHuman definitions, contexts, chat DTOs,
policies and artifact paths stay out of these types.

## TDD-ordered implementation

### Phase 0 — graph binding resume/retry prerequisite

**Owner/files:** TinyAgents graph owner: `vendor/tinyagents/crates/
tinyagents-graph/src/compiled/{executor.rs,test.rs}` and
`src/subgraph/{mod.rs,test.rs}`. This is mandatory before lifecycle cutover.

**RED:** Add a checkpointed parent graph with a `SubAgentNode` reached only
after an interrupt and after a failed node. A recording `AgentInvocationBinding`
must make identity of invoker/event sink/cancellation observable. Assert that
`resume_from_with_agent_binding(thread,target,command,binding)` and
`retry_with_agent_binding(thread,binding)` reach the resumed child with that
binding. Add the same tests through `shared_subgraph_node` and
`adapter_subgraph_node`, including a child which itself resumes. Verify live
cancellation/events still work and neither checkpoint JSON nor `CompiledGraph`
retains the binding.

**GREEN:** Centralize continuation in `resume_from_inner(...,
Option<AgentInvocationBinding>)`; thread it through `execute`, `execute_run`,
node-context construction, child graph construction and every resumed branch
of `subgraph::drive_child`. Plain resume/retry may remain unbound, but a
binding-aware route must never silently become unbound; fail closed at a
subagent node without a binding. Do not use a global cache or checkpoint field.

**Verify:**

```bash
cargo test --manifest-path vendor/tinyagents/Cargo.toml -p tinyagents-graph binding
cargo test --manifest-path vendor/tinyagents/Cargo.toml -p tinyagents-graph subgraph
```

### Phase 1 — queue direct imports (reserved for active Terra worker)

Do not edit `agent/harness/run_queue/**`, `agent/mod.rs`, or its in-progress
callers: a separate Terra worker owns this phase. Record/review its result
before Phase 2. Its required endpoint is deletion of the OpenHuman wrapper and
direct `tinyagents_harness::run_queue::{RunQueue, QueueLane, QueueStatus}` use
as `RunQueue<OpenHumanQueuedTurnDto>`. Keep `QueueMode`/DTO/request-mode logic
host-side: interrupt and parallel are caller behavior; steer/followup/collect
map to the three generic lanes. Migrate `agent_graph.rs`, session/subagent
callers, web-chat and orchestration steering to direct `push`, `drain`,
`status`, and `clear`; add no compatibility alias.

The acceptance suite covers lane FIFO, typed DTO retention, interrupt/parallel
not entering a queue, safe-boundary steer/collect, followup after terminal and
queue cleanup on cancellation. Require focused queue tests and
`cargo check --manifest-path Cargo.toml -p openhuman`.

### Phase 2 — layout migration into `tinyagents-session`

**RED:** Port `agent/harness/session/migration_tests.rs` to
`tinyagents-session/src/transcript/migration_test.rs`: marker idempotency,
fresh root, JSONL/Markdown layout conversion, collision preservation,
warning-only individual error, non-date directory preservation and pruning.

**GREEN:** Move code/tests, update `platform/startup/ops.rs` and every boot
caller to direct `tinyagents_session::transcript::migrate_layout_if_needed`,
then delete the OpenHuman module/export. Retain one OpenHuman startup
integration test proving the configured workspace triggers it once.

**Verify:** `cargo test --manifest-path vendor/tinyagents/Cargo.toml -p
tinyagents-session migration` and `pnpm debug rust session_migration`.

### Phase 3 — create and prove `tinyagents-runtime`

**RED:** With fake codec/hooks/locator/harness, test first turn, stable prefix
after history growth, immutable tool snapshot, full-fidelity resume, trailing
input dedup, append-only delta, compaction delta, interrupted partial save,
hook ordering/failure, harness error, and cancellation at every await. Assert
the terminal hook fires exactly once and a failed persist restores the prior
in-memory snapshot.

**GREEN:** Build the modules/API above using direct
`tinyagents_session::transcript::{TranscriptLocator,TranscriptHistory,
TranscriptTurn}` and harness invocation APIs. The driver has one state
transition and a drop-safe terminal guard. It accepts explicit `TurnOptions`
and `RunContext`, not task-local state. Add a dependency boundary test.

**Verify:**

```bash
cargo test --manifest-path vendor/tinyagents/Cargo.toml -p tinyagents-runtime
cargo test --manifest-path vendor/tinyagents/Cargo.toml -p tinyagents-session transcript
cargo check --manifest-path vendor/tinyagents/Cargo.toml --workspace
```

### Phase 4 — session host adapter and session deletion

**RED:** Preserve tests for builder/default/factory/provider role, definition,
memory-write, tool exposure/spec views/listener/autoload, transcript thread
resume/scoping/request-id/prefix/history, tool/reasoning/usage wire fidelity,
turn context/checkpoint/wrapup/failure/grounding/required output/recall, and
CLI/channel/embedder/cron/local cancellation paths.

**GREEN:** Create `crates/openhuman-core/src/agent/session_host/{mod,builder,
codec,hooks,factory,prompt,memory,policy,progress,finalize,resume,tests}.rs`.
Its codec converts `ChatMessage`; its hooks own prompt/memory/experience,
security, BUS/progress, post-turn work, usage and finalization. It creates the
runtime builder with explicit `OpenHumanRunContext`, resolved tool snapshot and
session locator. Migrate callers in `agent/mod.rs`,
`inference/host_runtime/ops/agent_chat.rs`, `web_chat/**`, `channels/**`,
`flows/**`, `skills/runtime/**`, `voice/realtime_harness/agent.rs`,
`openhuman-embed/**`, `openhuman-tui/**`, and session import. Delete the old
session tree, harness exports, stale README links and tests; locator/history
imports are direct session-crate imports.

**Verify:** `pnpm debug rust session`, `pnpm debug rust agent_turn`, `cargo
check --manifest-path Cargo.toml -p openhuman`, and `pnpm rust:layout`.

### Phase 5 — neutral subagent orchestration

**RED:** Fakes for planner/executor/persistence prove: planner rejection does
no work; prepared run context reaches execution; each completed/incomplete/
pause/cancel result performs exactly one terminal persistence action; save/load
errors are typed; duplicate task ids do not double-record; cancellation before,
during and after execute is truthful; and nested usage contributes once.

**GREEN:** Implement the `subagent` module/API above. Pause/checkpoint and
transcript records stay neutral. Add a session adapter only if it does not make
session name orchestration values; otherwise defer the adapter to Phase 6.

**Verify:** `cargo test --manifest-path vendor/tinyagents/Cargo.toml -p
tinyagents-orchestration subagent` and the full crate test suite.

### Phase 6 — subagent host adapter and runner deletion

**RED:** Port old runner behavior by seam: tier/definition/typed model/tool
filter/ranking/recovery tests; default/custom graph failure/checkpoint/pause/
resume/worker-mirror tests; queue steering/cancellation/explicit context/
spawn-depth/recency/sandbox/workspace tests; usage rollup and unique
progress/BUS event tests; and artifact/offload/handoff/error taxonomy tests.

**GREEN:** Create `crates/openhuman-core/src/agent/subagent_host/{mod,planner,
executor,persistence,definition,prompt,tools,graph,progress,artifacts,resume,
tests}.rs`. Planner resolves definition/tier/security/tool/model/memory/prompt
policy and emits a filtered immutable `PreparedSubagent`. Executor inherits
`OpenHumanRunContext`; provider/model routing, artifact paths, progress and
worker mirroring stay host-owned. Persistence implements OpenHuman checkpoint
file/session-DB behavior where no neutral session adapter exists. Migrate
`agent/orchestration/{ops.rs,delegation.rs,running_subagents/**,
spawn_parallel_graph/**,subagent_sessions/**,tools/**}`, `agent/triage/**`,
`agent/registry/**`, `agent/task_dispatcher/**`, `tools/orchestrator_tools.rs`
and integration/raw tests.

Use `tinyagents_harness::handoff::ResultHandoffCache` directly; only the
OpenHuman-configured size threshold remains at the host callsite. Move the
OpenHuman `ExtractFromResultTool` into `subagent_host`. Delete the complete old
runner tree and its exports in the same change.

**Verify:** `pnpm debug rust subagent`, `pnpm debug rust spawn_subagent`,
`pnpm debug rust continue_subagent`, `pnpm debug rust agent_harness`, and
`cargo check --manifest-path Cargo.toml -p openhuman`.

### Phase 7 — boundary gate and final deletion audit

Update agent/TinyAgents READMEs and architecture sources, run `pnpm
docs:generate`, then extend the established runtime-boundary checker to reject
deleted module declarations and forwarding surfaces. These final production
grep gates must be empty (negative checker fixtures are the sole exception):

```bash
rg -n 'agent::session_host|tinyagents_runtime::Session' crates tests
rg -n 'agent::harness::subagent_runner|harness::subagent_runner|mod subagent_runner;' crates tests
rg -n 'pub use .*tinyagents_(runtime|session|orchestration)|type .*=(.*tinyagents)' crates/openhuman-core/src
rg -n 'struct RunQueue|impl RunQueue|harness/run_queue' crates/openhuman-core/src/agent
rg -n 'ResultHandoffCache' crates/openhuman-core/src/agent/harness
```

Also prove no vendored manifest names `openhuman`, and no harness/session/graph
manifest depends on runtime or orchestration. Inspect `cargo metadata` against
the dependency diagram above.

## Validation and landing order

Use focused gates per phase. Before declaring complete, run long commands via
`scripts/ci-cancel-aware.sh` and do not export `CARGO_TARGET_DIR`:

```bash
scripts/ci-cancel-aware.sh cargo test --manifest-path vendor/tinyagents/Cargo.toml --workspace
scripts/ci-cancel-aware.sh cargo test --manifest-path Cargo.toml -p openhuman
cargo check --manifest-path Cargo.toml
pnpm rust:layout
pnpm docs:generate
pnpm docs:check
pnpm typecheck
pnpm lint
pnpm i18n:check
```

Land bottom-up: graph binding prerequisite; the separate queue direct-import
PR; session migration API; runtime crate; orchestration subagent API; OpenHuman
session host cutover; OpenHuman subagent host cutover; then docs/deletion gate.
Each vendored change needs a canonical-upstream PR before OpenHuman updates its
gitlink. Work only from the superproject worktree, preserve commits and never
squash.

# Moving `agent/` into TinyAgents

**Status:** spec + plan. Direction decided by the maintainer on 2026-07-28
after two rounds of pushback; this document plans the move rather than
re-arguing it. The open questions below are *how*, not *whether*.
**Scope:** relocate `src/openhuman/agent/` (152 files, 70,530 LOC) into
`vendor/tinyagents` as a generic agent runtime, with OpenHuman's coupling
expressed as trait injection.
**Supersedes:** the "permanent host" dispositions for `builder/`, `turn/`,
`runtime.rs`, and `types.rs` in
`2026-07-28-agent-session-transcript-to-tinyagents-design.md` §6, and the
`agent/ remainder — STAYS` row in `tinyagents-migration-plan-2026-07-22.md` §7.
Those rows are reopened by this decision and must be updated in the ledger.

---

## 1. Why this is feasible (the enabling fact)

The earlier objection was dependency direction: `agent/` reaches 45 OpenHuman
domains, so relocating it appeared to force a GPL-redistributed crate to import
Composio, `SecurityPolicy`, and `memory_store`.

That objection assumes the code moves *as written*. It does not have to,
because **the crate is already generic over a host-supplied state type**:

```rust
pub struct AgentHarness<State: Send + Sync, Ctx: Send + Sync = ()> { … }
pub trait Tool<State: Send + Sync>: Send + Sync { … }
pub trait ChatModel<State: Send + Sync>: Send + Sync { … }
pub trait Middleware<State: Send + Sync, Ctx: Send + Sync = ()>: Send + Sync { … }
```

`State` is the injection vehicle. A relocated agent runtime does not import
`crate::openhuman::memory` — it is generic over a `State` that *provides*
memory, and OpenHuman supplies the impl. The crate already ships **18 extension
traits** on exactly this pattern (`ChatModel`, `Tool`, `ChatHistory`, `Store`,
`AppendStore`, `Summarizer`, `EmbeddingModel`, `VectorStore`, `ResponseCache`,
`WorkspaceIsolation`, `HarnessEventJournal`, `HarnessStatusStore`,
`EventListener`, `Middleware`, `ModelMiddleware`, `ToolMiddleware`,
`ModelBaseCall`, `ToolBaseCall`). This move adds ~10 more of the same kind. It
is not a new architecture; it is more of the one already in use.

The GPL/crates.io concern also narrows correctly: the constraint is that no
*OpenHuman product logic* is published, not that no agent runtime is. Traits and
a generic loop are publishable; `provider_role_for`'s `subconscious` routing and
the `integrations_agent` dispatcher override are not — they become host impls.

---

## 2. Ground truth: the coupling to invert

### 2.1 Outbound — what `agent/` imports (45 domains)

By reference count:

| Refs | Domain | Becomes |
| ---: | --- | --- |
| 195 | `config` (`Config` 53, `AgentConfig` 45, `MemoryConfig` 28, `ContextConfig` 24, …) | **crate config structs**, populated host-side. The single largest blocker — see §4.1 |
| 91 | `tinyagents` (the seam) | dissolves — becomes internal |
| 84 | `tools` | existing `Tool<State>` + `SharedToolAdapter` |
| 72 | `inference` | existing `ChatModel<State>` + a `ModelResolver` trait |
| 57 + 41 + 16 + 11 + 5 + 3 | `memory`, `memory_store`, `memory_tree`, `agent_memory`, `memory_tools`, `memory_conversations` | **`MemoryProvider`** trait |
| 52 | `context` | **`ContextComposer`** trait |
| 34 + 22 | `profiles`, `agent_registry` | **`DefinitionRegistry`** trait |
| 28 | `composio` | host `Tool` impls — no new trait |
| 25 + 10 + 6 + 4 + 2 | `security`, `approval`, `agent_tool_policy`, `sandbox`, `prompt_injection` | **`SecurityGate`** trait |
| 22 + 1 | `skills`, `skill_runtime` | host `Tool` impls |
| 21 | `todos` | already crate `graph::todos` (parent spec DS-1) |
| 19 + 7 + 4 | `tokenjuice`, `cost`, `scheduler_gate` | **`BudgetGate`** trait |
| 11 | `learning` | **`LearningSink`** trait |
| 8 | `subconscious` | host impl behind `LearningSink` |
| 6 + 4 | `web_chat`, `channels` | **`ProgressSink`** trait |
| 5 | `tool_status` | **`ToolOutcomeClassifier`** trait |
| 5 | `thread_goals` | host impl behind `ContextComposer` |
| 5 | `embeddings` | existing `EmbeddingModel` |
| 5 | `agent_orchestration` | existing `graph::orchestration` |
| 3 | `agent_experience` | **`ExperienceStore`** trait |
| remainder (`util`, `app_state`, `session_db`, `file_state`, `task_sources`, `mcp_registry`, `threads`, `session_import`, `migrations`, `tinycortex`, `tool_timeout`) | ≤ 4 refs each | host impls or inlined generics |

**~10 new traits** cover 45 domains, because most domains reach `agent/` through
one of a few conceptual seams.

### 2.2 Inbound — what imports `agent/` (48 domains)

This is the half the earlier analysis under-weighted, and it is the larger risk.
By symbol:

| Refs | Symbol | Note |
| ---: | --- | --- |
| 275 | `agent::harness::*` | the bulk; moves down |
| 58 | `agent::turn_origin` | product enum — **stays host** |
| 43 | `agent::messages` (`ChatMessage`) | durable DTO — **stays host** (WP-1) |
| 24 | `agent::triage` | product — **stays host** |
| 22 | `agent::progress` (`AgentProgress`) | UI contract — **stays host**, produced via `ProgressSink` |
| 14 | `agent::prompts` | `SOUL.md`/`IDENTITY.md` — **stays host** |
| 14 | `agent::host_runtime` | **stays host** by definition |
| 14 | `agent::bus` | event-bus glue — **stays host** |
| 12 | `agent::task_board` | already crate `graph::todos` |
| 12 | `agent::message_convert` | boundary adapter — **stays host** |
| 11 | `agent::hooks` | trait defs move; impls stay |
| 8 | `agent::error`, 8 `agent::cost`, 7 `agent::tool_policy`, 7 `agent::progress_tracing`, 7 `agent::pformat`, 6 `agent::task_dispatcher`, 4 `agent::stop_hooks` | mixed; see §3 |

**Consequence:** `agent/` does not empty out. Roughly **20–25k LOC stays** as
the host adapter layer (`ChatMessage`, `AgentProgress`, `turn_origin`, prompts,
triage, bus, host_runtime, message_convert, the trait impls). The deliverable is
"the runtime moves down and OpenHuman keeps an adapter", not "the directory
disappears".

### 2.3 Honest cost

45 inbound domains to invert, 48 outbound consumers to repoint, ~29k LOC of
tests to migrate or re-home, a cross-repo change in two Cargo worlds, and an
on-disk/behavioural surface (transcript format, progress events, cost
accounting) that users depend on. **This is a multi-quarter program, not a
refactor.** §5 sequences it so every phase is independently valuable and the
program can be halted at any phase boundary without leaving the tree broken.

---

## 3. Disposition

### Moves into `tinyagents` (generic over `State`)

| Host area | Prod LOC | Lands as |
| --- | ---: | --- |
| `harness/session/{runtime,types,builder}` — session lifecycle & assembly | ~3,700 | `harness::session` — `Session<State>` + builder over capability traits |
| `harness/session/turn/*` — turn orchestration shell | ~4,476 | `harness::session::turn` — generic loop + `TurnPreparation` pipeline |
| `harness/subagent_runner/` | ~5,541 | merges into existing `harness::subagent` + `graph::orchestration` |
| `harness/session/transcript.rs` + `turn_checkpoint.rs` | ~2,100 | the crate **`Store`/`AppendStore` session journal** (`{workspace}/tinyagents_store/`), via the in-flight #4249 migration — **not** a `JsonlChatHistory`; corrected 2026-08-03, see §5 Phase 2 |
| `harness/{parse,definition,definition_loader,tool_filter,required_output,graph,agent_graph,fork_context}.rs` | ~3,300 | `harness::{tool_calling, definition, graph}` — merges with #55/#57 |
| `harness/artifact_offload/`, `tool_result_artifacts/` | ~1,400 | `harness::artifacts` — **`artifact_offload` landed** (tinyagents#101); `tool_result_artifacts/` still to do |
| `harness/run_queue/`, `harness/memory_context*.rs` | ~1,000 | `harness::runtime`, behind `MemoryProvider` |
| `task_dispatcher/`, `dispatcher.rs` (parse half), `pformat.rs`, `stop_hooks.rs`, `hooks.rs` (trait defs) | ~3,000 | `harness::{tool_calling, hooks}` |
| `progress_tracing/` | ~3,186 | deleted, not moved — crate observability already covers it (parent spec DS-5) |
| `multimodal.rs` (resolution half) | ~1,300 | `harness::multimodal` behind the `multimodal` cargo feature — **landed**, see §5 Phase 5 |

### Stays in OpenHuman as the adapter layer

`messages.rs` (`ChatMessage`), `message_convert.rs`, `progress.rs`
(`AgentProgress`), `turn_origin.rs`, `prompts/`, `triage/`, `bus.rs`,
`host_runtime.rs`, `error.rs`, `cost.rs`, `tool_policy.rs`,
`agent/tools/`, `archivist/`, `schemas.rs`, plus **every impl of the ~10 new
traits**. Estimated ~20–25k LOC including tests.

> **`multimodal.rs` moved off this list.** It read `multimodal.rs` until
> 2026-08-21, when the maintainer reversed that row and the module was split.
> The row was not wrong about the part that matters — the `ChatMessage`
> adapters, the config mapping, the proxy client, the PDF extractor and the
> on-disk attachment stash are all still here — it was wrong that *nothing*
> generic was underneath them. Marker parsing, `data:` URI decoding, MIME
> detection, the limit clamps and the rendered payload are the same for any
> host, and they are now `harness::multimodal`. The host file is ~1,300 lines
> lighter and reads as what §5 calls the irreducible remainder.

---

## 4. The two decisions that gate everything

### 4.1 Config (195 refs — the real blocker)

`agent/` reads `Config`, `AgentConfig` (742-line schema), `MemoryConfig`,
`ContextConfig` directly. A generic runtime cannot import OpenHuman's config
schema. Options:

- **A — Crate-owned config structs.** The crate defines `SessionConfig`,
  `TurnConfig`, `ToolConfig`; OpenHuman maps its schema into them at build time.
  Explicit, versionable, and mirrors how `MemoryConfig` is derived for TinyCortex
  (`tinycortex/config.rs::memory_config_from`). **Recommended.**
- **B — `ConfigProvider` trait** with ~40 getters. Avoids a mapping layer but
  turns every config read into a virtual call and makes the trait a dumping
  ground.
- **C — Generic `State` carries config.** Least code, worst discoverability;
  every crate-side read needs a bound.

Pick A. It is the pattern the org already uses successfully one crate over.

### 4.2 `ChatMessage` and the transcript format

Moving the session runtime down forces the durable conversation record to
become crate-owned, and `ChatMessage`'s durable fields must survive as crate
`Message` + a `raw` passthrough (the `ToolResult::raw` precedent). This is the
change with real user-visible risk — existing installs have live transcripts and
resume must keep working. Phase 2 exists solely to de-risk it.

> **Corrected 2026-08-03.** This section previously said the decision was
> "**Option B** of the transcript spec: the `session_raw` JSONL format becomes
> crate-owned public API". It is not. The in-flight migration (issue #4249,
> `src/openhuman/session_import/`) converges on the crate's **`Store` /
> `AppendStore` journal**, not on promoting the legacy JSONL layout to crate
> API. The legacy `session_raw/*.jsonl` format stays a host implementation
> detail and is retired once readers move; it never becomes public crate
> surface. See the Phase 2 note in §5.

---

## 5. Phased plan

Each phase is independently valuable and leaves the tree green. Stop-anywhere is
a hard requirement, not a nicety.

**Phase 0 — Ledger + trait catalogue (no code).** — *in progress (2026-08-02)*
Reopen the superseded rows (§ header). Write the ~10 trait signatures as an
upstream RFC in `vendor/tinyagents/docs/`. Nothing moves until the trait
catalogue is accepted upstream — otherwise the first mover defines the seams by
accident.
*Exit:* accepted RFC; ledger rows reopened.

Draft landed: [`docs/spec/host-capability-traits-rfc.md`](https://github.com/tinyhumansai/tinyagents/blob/main/docs/spec/host-capability-traits-rfc.md)
in the `tinyagents` repo (vendored here at `vendor/tinyagents/`; linked by URL
because the link checker does not check out submodules)
— all ten signatures, grounded in measured reference counts. **Not yet
accepted**; it carries four open questions that block Phase 1, the hard one
being a name collision: `tinyflows` 0.5.1 shipped its own, unrelated
`MemoryProvider` trait, and both crates are in OpenHuman's dependency graph.

**Phase 1 — Land the traits upstream, empty.**
Add the traits + no-op/in-memory default impls to the crate. No host change.
*Exit:* crate `cargo test --all-features` green; version bump; both lockfiles.

**Phase 2 — Transcript to the crate session store.** — *soak started
(2026-08-03)*
Do this early and alone: it is the only phase with on-disk risk. One release of
shadow-read parity, mismatch logged never panicked, legacy `DDMMYYYY/` and
`read_transcript_legacy_md` paths covered.
*Exit:* resume works across upgrade on a real workspace; parity soak clean.

> **This phase was mis-specified, and most of it was already built.** Two
> corrections, found on starting it:
>
> **1. The target is not `JsonlChatHistory`.** The heading previously read
> "Transcript to crate `JsonlChatHistory` (transcript spec Option B)". No such
> convergence is in progress. The real target — already chosen and half-shipped
> under issue #4249 — is the crate's `Store` / `AppendStore` journal at
> `{workspace}/tinyagents_store/{kv,journal}`. Building a `JsonlChatHistory`
> would have introduced a **third** store alongside the legacy JSONL and the
> one being migrated to.
>
> **2. It was ~2/3 done before this phase opened.** `src/openhuman/session_import/`
> (2,452 LOC) already implements:
>
> | Slice | State |
> | --- | --- |
> | Phase 1 — importer (legacy JSONL → store) | done |
> | 04.1 — live dual-write, `session_dual_write` | done, **defaults ON** |
> | shadow-read comparison + `ShadowReadOutcome` | done, was default OFF |
> | 04.2 — flip readers to the store | **not started** |
>
> Legacy `session_raw/*.jsonl` remains the authoritative reader *and* writer;
> the store is mirror-only. That is the correct sequencing and it was already
> right — this phase's job is to finish it, not restart it.

**Done this pass.**

- **Closed the two legacy-shape coverage gaps this phase's own exit criteria
  name**, neither of which had a test (`session_import/live_tests.rs`):
  - date-grouped `session_raw/DDMMYYYY/` resolves the same store stream as a
    flat transcript. The session key is the file *stem*, so the enclosing
    directory must not change it; if it ever did, every pre-migration session
    would read as `Unavailable` and the soak would look clean while covering
    nothing.
  - a legacy `.md` session reads as `Unavailable`, never `Divergence`. These
    predate the store, so no stream exists — reporting divergence would flood
    the soak with false positives from every old transcript on disk, and the
    point of the soak is that a warning means something.
- **`session_shadow_reads` now defaults ON**, starting the parity soak. Safe to
  default on because it is observation-only: legacy stays authoritative, the
  probe runs on a background task once per *resume* (not per turn), a store-read
  failure degrades to `Unavailable`, and `OPENHUMAN_SESSION_SHADOW_READS=0` is a
  kill switch. Worst case of a bad soak is log noise, not a broken resume.

**Remaining for Phase 2** — and the reason it is not yet done:

1. **Soak.** Collect `[session_shadow_read]` divergence rates from real
   workspaces across one release. There is no data yet, so nothing below is
   justified.
2. **04.2 — flip readers**, gated on the same flag, only once the soak is clean.
3. Retire the legacy writer once reads have run on the store for a release.

**Do not skip to 2.** The whole design of this phase is that the reader flip is
bought with evidence, and the evidence does not exist until a release has
shipped with the probe on.

**Phase 3 — Config mapping (§4.1 Option A).** — *in progress (2026-08-02)*
Introduce crate config structs + a host `session_config_from(&Config)` mapper.
Repoint `agent/` internals to the crate structs *in place*, before moving.
*Exit:* zero `crate::openhuman::config::` references inside the code slated to
move.

Landed so far — foundation only, nothing repointed yet:

- `tinyagents::harness::config` — `SessionConfig`, `TurnConfig`, `ToolConfig`,
  `MemoryLimits`, `RequiredOutput`, `ToolDispatcher`. Inert (serde + std only),
  defaults pinned to OpenHuman's current values.
- `src/openhuman/tinyagents/config.rs` — `session_config_from` plus
  `apply_team_models` / `apply_delegate`, following the
  `tinycortex::config::memory_config_from` precedent. Split three ways because
  OpenHuman's model pins are **not global**: `Config::teams` is keyed by team
  and `Config::agents` by delegate, so one flat mapper would have to invent the
  model for a session.
- 23 tests across both sides. The load-bearing one is
  `default_config_maps_to_the_crate_defaults`, which fails if the two default
  sets ever drift.

`ToolDispatcher` is an enum where the host has a `String`. The four accepted
spellings are `auto` / `native` / `xml` / `pformat` — **not** the
`auto`/`native`/`parsed` triple a reasonable person would guess. An unknown
value maps to `Auto` with a warning rather than failing: the host's own schema
lets a typo through validation, so refusing to build the session would turn a
cosmetic config error into an agent that cannot run.

### Repointing: what the "37 files" actually decomposes into

The 37-file figure counts every `agent/` production file with a qualified
`config::` path. **Only ~19 are in the moving set** — the rest (`host_runtime`,
`bus`, `triage/`, `schemas`, `multimodal`, `prompts/`, `agent/tools/`,
`archivist/`, `progress_tracing/`) stay host-side per §3 and *should* keep
reading `Config`; they are the mapper's callers. Repointing them would be
actively wrong. Within the moving set: **41 qualified refs**, which split into
three very different problems.

**1. Ambient config loads — 11 sites. Not a repoint; a signature refactor.**
*(Done 2026-08-02 — 11 sites → 2 genuine + 1 boundary snapshot.)*

`Config::load_or_init().await` appeared 11 times inside code slated to move.
A generic runtime has no config file and no `load_or_init`, so these could not
be pointed at a struct — the config had to be **threaded in from the caller**.

`load_or_init` is **not cached**: it re-resolves the config dirs and re-reads
`config.toml` on every call. `run_typed_mode` called it six times, so one
sub-agent spawn hit the disk six times and could observe six *different*
configs mid-spawn. `run_subagent` now takes a single snapshot
(`LoadedConfig = Result<Arc<Config>, String>`) and hands it down.

`Result<_, String>` rather than `Option` because the `integrations_agent` path
reports the load error to its caller while the other five degrade silently —
keeping both shapes lets each site preserve its original failure behaviour. The
snapshot is taken **after** `tier_gate_decision`: `load_or_init` can initialize
config on first run, and a spawn the tier gate rejects should not have that
side effect.

The sub-agent graph got the same treatment: `build_subagent_context_mw` now
takes `Option<&Config>` (and is no longer `async`), plumbed through
`run_subagent_via_graph` and a new `AgentTurnRequest::config` field so the
custom-graph path keeps its `[context]` knobs rather than silently falling back
to defaults. The four graph tests pass `None`, which makes them hermetic — they
previously read whatever `config.toml` was on the developer's machine.

**Scope correction: `task_dispatcher/` is not in the moving set.** §3 lists it
beside `dispatcher.rs`, both mapping to `harness::{tool_calling, hooks}`. That
conflates two unrelated modules. `dispatcher.rs` parses tool calls out of model
output and is genuinely generic. `task_dispatcher/` is a task-**card board**
dispatcher reaching `task_sources`, `threads`, `web_chat`, `todos`, `profiles`
and `scheduler_gate` — product logic that stays host-side. Its three
`load_or_init` calls are boundary code and are correct as they are. **§3's row
should be split.**

Two loads remain in moving files, both host-boundary code that gets extracted
rather than moved:

- `session/turn/tools.rs:123` — Composio integration fetch. Config is *already*
  threaded via the session's `runtime_config` (set in
  `factory.rs`); this is only the fallback when a session is built through
  the raw setter path. Composio is host product logic and becomes a `Tool` impl
  in Phase 4, so the fallback was left rather than risk silently disabling
  integration fetching for setter-built sessions.
- `harness/definition.rs:781` — `load_for_default_workspace()`, a convenience
  constructor with exactly one caller: `src/core/agent_cli.rs:415`. It is a CLI
  boundary helper that stays host-side when `definition.rs` moves.

**2. Blocked on Phase 2 — the session cannot drop `AgentConfig` yet.**

The session reads only **9 distinct `AgentConfig` fields**, 7 of which the crate
structs already cover. The two that do not — `session_dual_write` and
`session_shadow_reads` (`session/turn/session_io.rs`) — are *transcript*
live-store migration flags. They have no crate home until Phase 2 decides where
the transcript lives, so `Agent.config: AgentConfig` has to stay for now.

Adding a crate `SessionConfig` *alongside* it was considered and rejected:
`session/runtime.rs:181` mutates `self.config.max_tool_iterations` after build
(the iteration-cap override), so two configs would silently diverge on exactly
the field most read. One source of truth or none.

**3. Blocked on Phase 4 — `builder/factory.rs`.**

`factory.rs` reaches 21 domains and is going to be *split* into host trait impls,
not moved verbatim. Repointing its `Config` usage now is rework.

### Done in this pass

`RequiredOutputContract` → crate `RequiredOutput`, the one clean type swap
available: `harness/required_output.rs` (pure logic, no host domains) and
`session/turn/session_io.rs`, converting at the read site in
`session/turn/core.rs` via `tinyagents::config::required_output_from`. The
crate type gained `all_keys()` with semantics identical to the host's, including
the subtle one — a blank `block_key` makes the contract inert *even when
`required_keys` lists siblings*. The 12 existing `required_output` tests pass
unchanged against the crate type, which is the proof the swap is behaviour-
preserving.

The mapper was also split into per-section functions (`turn_config_from`,
`tool_config_from`, `memory_limits_from`, `apply_agent_config`) because the
session builder takes a **per-agent `AgentConfig` override** — mapping only from
the global `Config` would have discarded it and run every agent on the global
limits.

### Revised remaining work

1. ~~Thread config through the ambient-load sites.~~ **Done 2026-08-02.**
2. After Phase 2: replace `Agent.config` with crate config; migrate the two real
   external `agent_config()` consumers (`agent_orchestration/parent_context/`,
   `subconscious/session.rs`).
3. After Phase 4: `factory.rs`, and the Composio fetch in `session/turn/tools.rs`.
4. Split §3's `task_dispatcher/` + `dispatcher.rs` row — only the latter moves.

> **Test note.** `openhuman::agent::` needs `RUST_MIN_STACK=16777216` or
> `session::tests::turn_dispatches_spawn_subagent_through_full_path` overflows
> the stack (already flagged in §6). With it set, the suite is 1080 pass / 1 fail
> — `builder_tests::profile_allowed_tools_restrict_shared_session_builder` fails
> **on a clean tree too** when run with the full suite and passes in isolation, so
> it is a pre-existing order-dependence, not Phase 3 fallout.

**Phase 4 — Implement the traits host-side, still in place.** — *adapters
landed (2026-08-03); call sites not yet repointed*
`AgentMemory`, `ContextComposer`, `SecurityGate`, `BudgetGate`,
`DefinitionRegistry`, `ExperienceStore`, `LearningSink`, `ProgressSink`,
`ToolOutcomeClassifier`, `ModelResolver`. `agent/` calls them instead of
reaching into domains directly. **This phase delivers most of the architectural
value with none of the relocation risk** — after it, `agent/`'s outbound
coupling is ~10 traits instead of 45 domains, and the program can legitimately
stop here.
*Exit:* `grep -c "crate::openhuman::" src/openhuman/agent/harness/session/` down
from its baseline to the adapter layer only.

> **Exit-criterion baseline corrected.** The figure above read "~2,000 refs".
> Measured: **295** in `session/` production code (548 including tests). The
> larger number counted a wider tree. 295 is the number to drive down.

**Landed: all ten adapters** in `src/openhuman/tinyagents/host/`
(~6,000 LOC, 140 tests). Each wires one crate trait to the real OpenHuman
domains, with policy enforced adapter-side. `agent/` **does not call them yet**,
so the exit criterion is still at 295 — writing the adapters and repointing the
callers are two separate pieces of work and only the first is done.

Two defects were found and fixed at integration, both of the kind that compiles
cleanly and fails silently:

- **`security_gate`: a channel `RequireApproval` verdict was resolving to
  `Allow`.** The mapping returned "no verdict" on the theory the call would fall
  through to the approval park — but the park is reached only from the `shell`
  and external-effect branches, so any ordinary tool was authorized with nobody
  asked. `agent_tool_policy::engine` files `RequireApproval` under
  `blocked_tool_names` alongside `Deny`, so this inverted the host's own
  semantics. Latent only because `build_session` does not currently emit
  `RequireApproval` — i.e. a trap armed for whoever turns it on. Now routed to
  the park, and **denied** when no approval gate exists (the one place this
  adapter set denies where the legacy middleware allows, argued in the module
  header). Pinned by `require_approval_never_silently_allows_a_plain_tool`.
- **`experience_store`: cross-agent record collision.** The domain's
  `stable_experience_id_for_profile` hashes task + tool sequence + outcome +
  profile and deliberately **excludes `agent_id`**; the native capture hook is
  protected only incidentally, by always supplying a real tool sequence. This
  adapter has none to supply, so two agents recording the same task with the
  same outcome collided on one id and `put` upserted — the second writer
  silently destroying the first's record. The agent id is now folded into the
  hashed tool-sequence slot.

**Remaining for Phase 4 — repointing. Investigated 2026-08-04 and found
blocked, not merely hard.** Four findings, each measured:

**1. The exit criterion counts the wrong code.** Of the 118 non-adapter-layer
refs in `session/`, roughly two-thirds are not consumption at all:

| Where | Refs | What it is |
| --- | ---: | --- |
| `builder/` (factory 29, setters 12, mod 3, helpers 2) | 46 | assembly — becomes the trait *impls*, per §3 |
| `types.rs` | 16 | field type annotations — the injection points themselves |
| `runtime.rs` | 13 | session state management (e.g. `rebuild_tool_policy_session`) |
| `turn/` | 40 | the only genuine runtime consumption |
| misc | 3 | |

Driving `session/` "down to the adapter layer only" therefore cannot happen by
repointing: most of those refs **are** the adapter layer. The honest metric is
`turn/`'s ~40.

**2. The session distributes handles more than it consumes them.** All 11 uses
of `self.memory` hand the `Arc<dyn Memory>` to a collaborator (the memory
loader, the context loader, `AgentExperienceStore`) rather than calling recall
or store. Swapping the field to `Arc<dyn AgentMemory>` would break those
collaborators, which need the full domain interface. The memory seam cannot be
repointed until they move behind traits too.

**3. No capability trait fits an existing call shape 1:1.** Checked all ten.
`ContextComposer::compose_system_prompt` returns a `String`, but the turn needs
structured `LearnedContextData` to feed its own `SystemPromptBuilder`, so
adopting it means moving the whole prompt assembly into the adapter — the same
work as Phase 5, not a repoint. `ToolOutcomeClassifier`'s only host consumer is
`progress_tracing/`, which §3 **deletes** rather than moves. `subconscious` in
`session/` is type annotations in `factory.rs` only.

**4. Four adapters cannot be constructed from session state.**

| Adapter | Constructor needs | Session has |
| --- | --- | --- |
| `AgentMemory`, `ExperienceStore` | `Arc<dyn Memory>` | yes (`memory_arc()`) |
| `ToolOutcomeClassifier` | nothing | yes |
| `ProgressSink` | `Sender<AgentProgress>` | yes |
| `LearningSink` | `Vec<Arc<dyn PostTurnHook>>` | yes |
| **`BudgetGate`, `ContextComposer`, `ModelResolver`** | **`Arc<Config>`** | **yes — the session carries `runtime_config: Option<Arc<Config>>`** |
| **`SecurityGate`** | **`Arc<SecurityPolicy>`** + tool sets | tool sets yes, **policy no** |

The `Arc<Config>` gap that used to head this chain is **closed**: the session
field was promoted from `integration_runtime_config: Option<Config>` to
`runtime_config: Option<Arc<Config>>` in #5396, and the same pass collapsed the
subagent spawn path from seven ambient `Config::load_or_init()` calls to one.
`BudgetGate`, `ContextComposer` and `ModelResolver` are therefore constructible
today.

What remains blocked is narrower than it was:

> Phase 4's **repointing** still needs Phase 2 to rehome
> `session_dual_write` / `session_shadow_reads`, which needs the parity soak,
> and the soak needs a *shipped release* to produce data.
>
> So §5's claim that Phase 4 "delivers most of the architectural value with
> none of the relocation risk" holds for the **adapters**, which are done. The
> repointing half is gated on elapsed time, not on effort.

**Two further blockers surfaced in #5396's review, both crate-side** and both
filed upstream — repointing should not proceed past them:

- [`tinyagents#88`](https://github.com/tinyhumansai/tinyagents/issues/88) —
  `ProgressEvent` has no tool-completion milestone, so a host cannot report a
  tool's outcome. Tool rows would stay `running` forever; synthesising
  `success: true` would put wrong data in the timeline and the trace exporter.
- [`tinyagents#89`](https://github.com/tinyhumansai/tinyagents/issues/89) —
  `ModelResolveRequest` carries no model pin, so a definition's exact model id
  has no route to the resolver. Per-agent pins would silently resolve to the
  workload default.

> **Both are fixed upstream in
> [`tinyagents#100`](https://github.com/tinyhumansai/tinyagents/pull/100)**,
> awaiting merge. `ProgressEvent` gains `ToolCallFinished { run, call, success,
> output }` and `ModelResolveRequest` gains `model_pin: Option<String>`.
>
> Two notes for whoever does the repointing:
>
> - **Neither type is emitted by the runtime yet.** The PR makes the seams
>   *expressible*; the agent loop still has to produce `ToolCallFinished`, and
>   the subagent runner still has to populate `model_pin` from
>   `AgentDefinition.model`. Landing #100 unblocks the adapters, it does not
>   wire them.
> - **The `model_pin` is advisory by design.** The host decides whether it can
>   honour a pin, because the runtime has no view of credentials or provider
>   health. `OpenHumanModelResolver` should therefore validate the id against
>   configured providers rather than pass it through blind — which is the
>   behaviour §4's adapter notes already wanted and had no channel for.

**21 `TODO(phase4)` markers** remain across the adapters, each naming a domain
surface that was not reachable. They are honest gaps, not stubs pretending to
work; the notable ones are `AgentMemory::thread_summary` (no host-authored
per-thread prose rollup exists) and `SecurityGate::screen_input` never returning
`Redacted` (OpenHuman can detect PII but exposes no public text-rewriting
helper).

**Phase 5 — Relocate, module family at a time.** — *3 of 7 families landed*
Order by inbound coupling, lowest first: `artifact_offload` → `run_queue` →
`multimodal` → `parse`/`tool_calling` (merges with DS-5b) → `subagent_runner` →
`session/turn` → `session/{builder,runtime,types}`. Each family: move to
`vendor/tinyagents/src/harness/`, re-export from the host adapter for one
release, then repoint consumers.
*Exit per family:* crate tests green; host `cargo check` both worlds; the
family's tests live upstream.

| Family | State |
| --- | --- |
| `artifact_offload` | **landed** — crate `harness::artifacts` (tinyagents#101), host keeps the prompt contract + two policy adapters |
| `run_queue` | **landed** — crate `harness::run_queue` owns the FIFO mechanics; host wrapper keeps `QueueMode::{Interrupt,Parallel}` and the `QueuedMessage` payload |
| `multimodal` | **landed** — crate `harness::multimodal` owns marker parsing, `data:` URIs, MIME detection, limits and payload rendering; host keeps the `ChatMessage` adapters, the config mapping, the proxy client, the `documents` PDF extractor and the attachment stash |
| `parse`/`tool_calling` | not started |
| `subagent_runner` | not started |
| `session/turn` | blocked on Phase 2's soak |
| `session/{builder,runtime,types}` | blocked on Phase 2's soak |

> **The shape both landed families converge on**, and the template for the
> rest: the crate owns the mechanics, the host keeps a thin wrapper holding
> exactly the parts that are product-specific. `run_queue` keeps two enum
> variants and a payload struct; `artifact_offload` keeps prompt text and two
> policy adapters. Neither host module is a re-export shim — each is the
> irreducible remainder, which is what "keep just the wiring" means in
> practice.
>
> **Two rules the `artifact_offload` move established**, both of which will
> recur:
>
> 1. **Prompt text never moves.** §6 lists OpenHuman prompt text as
>    unpublishable, and prompts name host tool ids. So the crate's pointer
>    renderer takes the read-tool name as a *parameter* rather than a constant
>    — a hard-coded tool name in a redistributed crate would put a tool some
>    other host does not have into its prompts.
> 2. **A host policy becomes a trait pair, not an import.** `SecurityPolicy`
>    and `sanitize_text` became `ArtifactPathPolicy` / `ArtifactRedactor`. The
>    host constructor installs both every time, because the crate permits
>    `None` for each and OpenHuman never wants either — routing construction
>    through one helper is what stops a call site producing an unguarded writer
>    by omission.

**Phase 6 — Collapse the seam and the adapter.**
`src/openhuman/tinyagents/` dissolves into the host adapter layer. Delete the
compatibility re-exports.
*Exit:* `agent/` is the adapter layer only; parent spec's DS-0 re-export gate
allowlist is seam-free.

**Phase 7 — Exit gate.**
Full `scripts/test-rust-with-mock.sh`, `cargo test --all-features` in both
vendored crates, slim disabled build **and** `cargo test --lib
--no-default-features core::`, `pnpm
rust:check`, deletion-ledger totals reconciled, architecture docs rewritten.

---

## 6. Risks

- **Inbound coupling is the real cost, not outbound.** 48 domains import
  `agent::`. Phase 5's per-family re-export window is what keeps that tractable;
  skipping it turns every family move into a 48-domain atomic commit.
- **On-disk transcript risk (Phase 2)** is the only user-visible data risk in
  the program. It is deliberately isolated and sequenced first.
- **`AgentProgress` is a UI contract.** It stays host-side and is produced
  through `ProgressSink`. If it drifts into the crate, the frontend timeline,
  cost footer, and citation chips break in ways unit tests will not catch.
- **Trait-explosion.** Ten traits is the budget. If Phase 4 needs a fifteenth,
  that is a signal a seam is wrong — re-open the RFC rather than adding it.
- **GPL/crates.io.** Publishable: traits, generic loop, tool-calling wire
  formats. Not publishable: `provider_role_for`'s `subconscious` routing, the
  `integrations_agent` override, OpenHuman prompt text, backend phrasing, key
  material. Every relocated file needs this check.
- **≥ 80% diff-coverage gate** on a program of this size — Phases 4 and 5 touch
  hundreds of files. Check `diff-cover` per slice.
- **Two Cargo worlds** — every crate bump regenerates root and
  `app/src-tauri` lockfiles (#3877).
- **`RUST_MIN_STACK=16777216`** — the subagent runner's large futures already
  overflow the default stack on Apple Silicon; Phase 5's subagent move is
  exactly where that resurfaces.
- **`GGML_NATIVE=OFF`** for local root-crate builds.

---

## 7. Summary

| | |
| --- | --- |
| Decision | move `agent/` into `tinyagents` (maintainer call, 2026-07-28) |
| Enabler | the crate is already generic over `State`; 18 extension traits use the pattern today |
| Inversion | 45 outbound domains → **~10 capability traits** |
| Reality check | `agent/` does not empty — ~20–25k LOC stays as the host adapter (`ChatMessage`, `AgentProgress`, prompts, triage, bus, trait impls) |
| Gating decisions | config mapping (§4.1 → Option A); durable conversation record converges on the crate **store**, not a crate-owned JSONL (§4.2, corrected 2026-08-03) |
| Highest-value / lowest-risk phase | **Phase 4** — trait injection in place. Cuts coupling 45 → 10 without moving a file; a legitimate stopping point |
| Highest-risk phase | **Phase 2** — on-disk transcript format, isolated and sequenced first. Was mis-specified and already ~2/3 built under #4249; parity soak started 2026-08-03 |
| Honest cost | multi-quarter program; every phase leaves the tree green and shippable |


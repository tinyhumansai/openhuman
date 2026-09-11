# Removing `tinymemory-core` and `tinycortex` from OpenHuman

Goal: the host links **`tinymemory-api` only**. All memory behaviour reaches the
loaded TinyMemory TinyBus module through `MemoryProvider`; nothing in `src/`
names an engine crate.

Spans three repos: `openhuman` (host), `vendor/tinymemory`, `vendor/tinycortex`.
Branch `tinymemory-bus-only` in each.

## Measured starting point (2026-08-30)

- `modules::registry` pins tinymemory **v1.13.3** — 144 bus members, 23
  capability families, all implemented by `ModuleMemoryProvider`. The in-tree
  docs claiming v1.0.1/v1.2.0 and a narrow seam are stale.
- `DriverClass::Embedded` is refused (`binding.rs:252`), so every
  `MemoryProvider` call is already over the bus.
- Production files naming `tinymemory_core::`: **23**. Naming `tinycortex::`: **~45**.
- Deleting the six engine glob re-exports yields **40 compile errors in ~30 files**.
  That is the true host coupling.

## The three clusters, and where each goes

### A. Composio providers (~20 files, 8.8k LOC in `tinymemory-core`)

Consumed mostly by `integrations/composio`, `flows` and `task_sources` — none of
which is memory. Split by what the thing *is*:

| Part | Destination | Why |
| --- | --- | --- |
| `tool_scope`, `catalogs*`, `descriptions`, `scope_lookup`, `capability_matrix`, `is_action_visible_with_pref` | **`tinymemory-bus::composio`** | static tables + pure functions, no I/O, no runtime. `ToolScope`/`UserScopePref`/`CuratedTool` types are already there; `tool_scope.rs` is a 32-line re-export of them. |
| `user_scopes::load_or_default` | bus `KvGet`/`KvPut` via `MemoryGraph` | it is a KV read (`crate::store::MemoryClientRef`), not a table. |
| `ComposioProvider` trait, `registry`, `ProviderContext`, provider impls (github/gmail/linear/notion/slack/clickup), `profile`, `profile_md`, `sync_state`, `periodic` | **stays sync-side** (tinycortex/module) | this is the syncing half. Host reaches it through `MemorySourceSync`. |

Host `get_provider(..).curated_tools()` becomes `catalog_for_toolkit(..)` from
the contract, which removes most `get_provider` call sites outright. The ones
that remain are real sync behaviour (`fetch_tasks`, `list_databases`,
`fetch_user_profile`) and need bus members.

### B. Tree (~9 files) — becomes graph/recall/retrieval via the API

Do **not** ask upstream for tree-shaped twins. Map onto the existing families:

| Host call today | Goes to |
| --- | --- |
| `tree::retrieval::{fast_retrieve, FastRetrieveOptions, QueryResponse}` | `MemoryRetrieval::fast_retrieve` |
| `tree::retrieval::source::query_source_scoped` | `MemoryTree::query_source` / `MemoryRetrieval::retrieve_source` |
| `tree::retrieval::types::NodeKind` | `RetrievalNodeKind` |
| `tree::score::extract::EntityKind` | `provider::retrieval::EntityKind` |
| `tree::health::async_run_doctor` | `MemoryMaintenance::doctor` |
| `tree::score::DEFAULT_DROP_THRESHOLD` consumer | `MemoryProfile::drop_facets_below` |

**`summarise` does NOT come home — corrected 2026-08-30.** The first reading of
this was "it builds an LLM chat provider, and the host owns inference, so it is
host policy like `source_scope`". That is wrong, and the evidence is in
`modules/memory_host.rs:45,406`: the **`ChatHost` seam already crosses the bus**
(`ai.tinyhumans.tinymemory.ChatHost`), so the module can call the host's chat
without the host owning the summariser. And `summarise` is not just a chat call
— it is `prepare_summary_prompt` + `finish_provider_summary` +
`fallback_summary` from `engine::backend::tree`, which know the summary tree's
format and belong with the tree. Bringing them home would mean two copies of the
tree's own prompt and parser, one of which the module keeps using.

So this is an **upstream ask**, not a relocation: a summarise member taking
`SummaryInput`s and a `SummaryContext`, so `agent/harness/archivist/recap.rs`
can ask the module for a recap instead of linking the engine to build the
prompt. `tree_runtime::engine::{run_summarization, rebuild_tree}` is the same
shape (it drives `tinyagents::ChatModel` over the tree's own machinery).

Genuinely missing from the contract, so upstream asks:

- `MemoryEntities::entity_score(id)` — replaces `tree::score::store::get_score`
- `DEFAULT_DROP_THRESHOLD` as a contract constant
- `MemoryTree` extension for the node store: `read_node`, `read_children`,
  `tree_status`, `write_node`, `buffer_write`, namespace/node-id validation
  (`tree_runtime::store::*`)
- **A staged diagnostic report.** `MemoryMaintenance::doctor` looked like the
  twin for `tree::health::async_run_doctor` and is not, so `memory/tools/doctor.rs`
  was deliberately **not** migrated. The contract's `MaintenanceReport` is
  `{operation, examined, changed, findings: Vec<String>}`; the engine's
  `DoctorReport` is `{healthy, stages: Vec<StageHealth>, first_blocking_cause:
  Option<PipelineFailure>, degraded: DegradedState, counters: DoctorCounters}`.
  The whole point of the `memory_doctor` tool is the per-stage health and the
  single first blocking cause, and `findings: Vec<String>` cannot carry either
  without the model parsing prose back into structure. Swapping would compile,
  return a plausible report, and quietly make the tool useless — the same shape
  of trap as `recall_namespace_scored` vs recency recall. The ask is a
  `DiagnosticReport` on `MemoryMaintenance` carrying stages and a blocking
  cause.

### C. Sources (~4 files) — a crate the host can simply take

This one needs **no bus work and no upstream change**, which the first pass
missed. `tinymemory_core::sources` is a thin layer over a separate crate:

- `sources::types` is already `pub use tinymemory_sources::{…}`.
- `sources::registry` is "the host's config path + a write lock" around
  `tinymemory_sources::registry::SourceRegistry::new(config.config_path())` —
  CRUD over the host's own `sources.toml`. The host owns that file; it does not
  need the engine to read it.
- `sources::readers` re-implements the reader dispatch over
  `tinymemory_core::Config`, where `tinymemory_sources::readers` takes a plain
  `workspace: &Path`.

`tinymemory-sources` costs serde, schemars, serde_json, anyhow,
`tinymemory-api`, async-trait, futures, regex, reqwest, walkdir, tracing,
chrono, tokio, toml, uuid — every one of which the host already has. No
`rusqlite`, no `tinycortex`, no engine.

So: depend on `tinymemory-sources` directly and let `memory::sources` own the
config-path and lock layer itself.

**Read `tinymemory_sources::readers`' module docs before touching `reader_for`.**
It returns `None` for the network kinds *deliberately* — "a network reader is
constructed explicitly by a caller that has already decided the fetch is
allowed; it is never handed out by the kind-dispatch that the workspace sync
loop drives on a timer", which is what keeps the host in charge of egress,
OAuth and cost. The engine's `reader_for` hands out all seven, so a literal
port would quietly move that decision. `composio` and `twitter_query` have no
reader in the crate at all (credentialed OAuth pipeline; unimplemented).

### D. `tinycortex` direct calls — the hard one

`tinycortex::memory::conversations::{list_threads, get_messages, ensure_thread,
append_message, purge_threads, ConversationMessage, CreateConversationThread}`
has **no contract family at all**. Callers: `memory/conversations/bus.rs`,
`memory/conversations/blocking.rs`, `channels/host/adapters.rs`, `threads/`.
Upstream ask: a `MemoryConversations` family.

Also engine-shaped: `memory::archivist::store::{session_entries, record_turn}`
(→ `MemoryEpisodic`, mostly covered), `memory::tree::{compile_flavoured_root,
flavoured_root_abs_path, get_tree_by_scope}` (`memory/tools/flavour.rs`),
`memory::persona::PersonaFacet` (→ `ProfileFacet`),
`memory::ingest::canonicalize::{chat, document}`.

## Ordering (each step must leave the tree green)

1. **tinymemory-bus**: move the static composio catalog/scope surface into the
   contract. No release coupling — it is a compile-time crate.
2. **Host**: repoint `integrations/composio`, `flows`, `task_sources` at the
   contract; delete `memory::sync::composio` re-export shims that are now dead.
3. **Host**: bring `summarise` and the tree-runtime summarisation home.
4. **Host**: repoint the tree/sources clusters onto retrieval/graph/recall.
5. **Upstream tinymemory/tinycortex**: `MemoryConversations`, the `MemoryTree`
   node-store extension, `entity_score`. Release; re-pin `modules::registry`.
6. **Host**: last engine callers; delete `host_impls` seam installation
   (`core/runtime/context.rs:616`), bring `thread_context` and
   `learning_candidate` home **in the same commit** as the engine's removal.
7. Drop both crates from `Cargo.toml` (deps and dev-deps) and both `[patch]`
   blocks; delete `direct_engine_refs_tests` once it can only be empty.

### Two traps that must not be split across commits

- **`thread_context`** is a `tokio::task_local!`. `tinymemory_core::store::recall_policy.rs:58`
  reads it. Two `task_local!` invocations are two keys, and unset means
  *exclude nothing* — recall would silently start echoing the caller's own
  thread back. It comes home **with** the engine, not before.
- **`learning_candidate::global()`** is a process-global ring buffer that the
  engine's `sync/composio/providers/profile.rs:129` pushes into. Moving it home
  while `sync::composio` still resolves into the engine gives two buffers and a
  silently empty one.


## Landed so far on `tinymemory-bus-only`

Baseline was green; each step below left `cargo check --lib` green.

**`tinymemory-bus`** — the curated Composio catalogs moved into the contract:
`catalogs/{business,google,messaging,microsoft,productivity,social_media}.rs`
plus the five provider-colocated tables (`gmail`, `notion`, `github`, `linear`,
`clickup`), `descriptions.rs`, and a new `catalogs/mod.rs` carrying
`catalog_for_toolkit`, `is_action_visible_with_pref`, `curated_scope_for`,
`toolkit_has_scope`, `CAPABILITY_TOOLKITS`, `NATIVE_PROVIDERS` and the
sync-interval helpers. `FastRetrieveQuery` gained a `Default` matching the
engine's.

**`tinymemory-api`** — `host::composio::capability_matrix()`, which needs
`ComposioCapability` and so cannot live in the bus crate.

**`tinymemory-core`** — `catalogs.rs`, `descriptions.rs`, `scope_lookup.rs` and
the five `<provider>/tools.rs` files are now re-exports of the contract, so the
engine and the host read one table. A drift guard in
`tree/retrieval/fast_tests.rs` pins the two `Default`s together.

**Host** — `integrations/composio/providers` stopped being a glob over the
engine and is split by what each half is: the catalog/scope/capability surface
from the contract, the provider registry and run types still from the engine and
listed by name. Every host call site moved off
`memory::sync::composio::providers::…` onto that path, so the shim is the only
file naming the engine's composio tree. All three `fast_retrieve` callers
(`memory/agent/ops.rs`, `memory/schema/handlers.rs`,
`agent/harness/subagent_runner/ops/runner.rs`) now go through
`binding.provider().as_retrieval()` with an explicit `as_bus_scope()`, and the
runner's test fixtures dropped their `tinycortex::` import.

### A pre-existing bug the new tests caught

`toolkit_description` keyed on `google_calendar` / `google_docs` /
`google_drive` / `google_sheets` / `onedrive`, while `CAPABILITY_TOOLKITS` and
`catalog_for_toolkit` key on the underscore-free slugs. Five toolkits — the four
Google ones and OneDrive — had been rendering the generic
"Interact with this connected service via its available actions" fallback in the
capability matrix and in the orchestrator's connected-integration prompt. Fixed
by aliasing; `toolkit_description_is_populated_for_every_capability_toolkit`
pins it.


## The sources migration (landed)

`memory::sources` went from `pub use tinymemory_core::sources::*;` to **one**
named engine line.

- `Cargo.toml` takes `tinymemory-sources` directly (`features = ["network"]`,
  matching what `tinymemory-core` enables).
- `memory/sources/registry.rs` — the config-path + write-lock layer, ported
  function for function over `tinymemory_sources::registry::SourceRegistry`.
  Same locking, same error stringification, same on-disk format. The `_in`
  variants keep taking an explicit config, because reading the process-global
  path from a workspace-bound caller is the cross-workspace leak the binding map
  exists to prevent.
- `memory/sources/readers/` — the `SourceReader` trait (product-shaped, takes a
  `&Config`) plus seven readers. Five are adapters over
  `tinymemory_sources::readers`; `composio` and `twitter` came home from the
  engine unchanged, with their tests.
- `memory/sources/types` re-exports the crate's vocabulary.

Still the engine's, named rather than globbed: `sync`, `status`, `reconcile`.

**`MemoryChunks::source_totals` is not a substitute for `status`.** It looks
like one. `SourceTotal` carries `chunk_count` and `most_recent_ms` but **no
`chunks_pending`** — and pending ("has no embedding, was not dropped, was not
skipped for re-embed") is the whole point of a sync-status row. It also omits a
source with zero chunks entirely, where `status_list` returns a row per
*configured* source. Migrating onto it would compile and quietly report a
healthy store. Upstream ask: a pending count on `SourceTotal`.

### `reader_for` hands out network readers — do not reuse it from a loop

`tinymemory_sources::readers::reader_for` returns `None` for the network kinds
deliberately, so that the host stays in charge of egress, OAuth and cost. The
host's `reader_for` hands out all seven, matching the engine's, because its
callers are RPC handlers acting on an explicit user request naming one source
id. That is written into the module docs. A polling loop must construct a
network reader deliberately instead.

## The kernel-floor ratchet is already red on `main`

`scripts/check-kernel-floor.sh` fails with **289 packages / 271 names against a
288 / 270 limit**, and it fails **identically on `main`** — verified by running
it in both checkouts and diffing the resolved package lists, which are the same
271 names with and without this branch's `tinymemory-sources` line. That
dependency was already in the kernel graph transitively through
`tinymemory-core`, so taking it directly costs nothing.

Do **not** raise `scripts/kernel-floor.limits` as part of this work: the growth
is not this branch's, and raising it here would launder someone else's
regression into a memory-migration PR. It needs finding and justifying on its
own.


## Are `sync` / `status` / `reconcile` already on the contract? (answered)

Mostly yes — an earlier note in this file said otherwise and compared against
the wrong type. Corrected:

| Engine module | On the contract? |
| --- | --- |
| **`sync`** | **Yes.** `MemorySourceSync::run_source_sync(source_id)` and `run_connection_sync(toolkit, connection_id)`. And the host barely uses this module: `sources::sync::sync_source` has **zero** call sites in `src/`; the only thing reached is `derive_scopes`, a pure helper over a `MemorySourceEntry` + `Config`. |
| **`status`** | **Per-provider: yes, and already wired.** `MemorySourceSync::sync_statuses()` returns `SourceSyncStatus { provider, chunks_synced, chunks_pending, batch_total, batch_processed, last_chunk_at_ms, freshness }`, and `memory/sync/sync_status/rpc.rs` already calls it through `binding.provider().as_source_sync()`. **Per-source: no.** `sources::status::status_list` is keyed by `source_id` and returns a row per *configured* source, including ones holding zero chunks. |
| **`reconcile`** | **Not a member, but not a bus gap.** `ensure_composio_sources` is `composio::scan_active_sync_targets` (a Composio API call) plus a registry batch upsert. The registry half is host-side already; the scan is Composio client work. This is a relocation, not an upstream ask. |

**Correction to the earlier entry in this document.** It claimed status was
inexpressible because `SourceTotal` carries no `chunks_pending`. That is true of
`SourceTotal` and irrelevant: `MemoryChunks::source_totals` is a chunk-grouping
read, not the sync-status twin. `SourceSyncStatus` is the twin and it *does*
carry `chunks_pending`. The real gap is narrower than stated — the key
(`provider` vs `source_id`) and the zero-chunk rows, not the pending count.

The engine's own `status.rs` says the pending predicate is "the engine's own
predicate from `list_sync_statuses`, kept identical so the per-source view and
the per-provider one cannot disagree about the same chunk" — i.e. these are two
deliberate views of one truth, and only one of them has a contract member.

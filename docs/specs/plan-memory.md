# Memory Subsystem — Pluggable Provider API & TinyCortex Consolidation

**Status:** proposed · **Date:** 2026-07-28 · **Scope:** `src/openhuman/memory*`, `src/openhuman/tinycortex/`, `vendor/tinycortex`
**Depends on:** [`kernel.md`](kernel.md)
**Supersedes (in disposition only):** `docs/tinycortex-cutover-evaluation-2026-07-28.md` §"Audit result" — see §6.1.

---

## 1. Goals

1. **Memory becomes an API, not an implementation.** The kernel owns a versioned memory contract;
   TinyCortex becomes the default *embedded driver* behind it, and a third-party backend
   (Supermemory, mem0, a self-hosted service) can be bound instead without touching kernel code.
2. **Everything TinyCortex-specific lives in TinyCortex.** The remaining engine-shaped host
   modules and re-export shims move into `vendor/tinycortex`. The host keeps only what a
   Supermemory-only build would still need: RPC, agent tools, policy, credentials, scheduling,
   registry, adapters.
3. **No behaviour change for the default build.** Bind `tinycortex`, and the RPC surface, agent
   tools, on-disk workspace, and parity harness are byte-identical to today.

## 2. Where we are

- 74k LOC across `memory` (18.3k), `memory_store` (17.4k), `memory_sync` (17.2k), `memory_tree`
  (9.9k), `memory_sources` (5.0k), `memory_diff` (2.0k), `memory_queue` (1.4k), `memory_tools`
  (1.3k), `memory_goals` (0.9k), `memory_conversations` (0.9k), `memory_search` (0.7k), plus a
  3.2k-LOC seam at `src/openhuman/tinycortex/`.
- The engine cutover is **complete**: TinyCortex is already the implementation authority for
  chunks, content, vectors, trees, retrieval, scoring, queue, ingest, readers, sync, diffs, goals,
  graph, conversations, and tool memory.
- But the coupling is **static and direct**. `memory::traits` is a `pub use tinycortex::memory::{…}`;
  `memory/global.rs` hands out a concrete `MemoryClient`; `UnifiedMemory` (the ten-table
  namespace-document tier) is host-owned SQLite. There is no seam a second backend can enter, and
  ~51 controller schemas assume the full capability set is present.

### What OpenClaw does, and what we take from it

OpenClaw's memory is a **single slot** (`plugins.slots.memory`) with a local default (Markdown +
sqlite-vec + FTS5 hybrid). Installing `memory-lancedb` or `@mem0/openclaw-mem0` claims the slot and
disables the incumbent with a warning. Providers expose a small tool triple —
`memory_recall` / `memory_store` / `memory_forget` — plus opt-in **auto-recall before the turn**
and **auto-capture after the turn**, bounded by `recallMaxChars` / `captureMaxChars` and a context
token budget. Ownership isolation is a storage predicate, not a post-search filter.

**Adopt:** one-slot binding; the recall/store/forget core triple; auto-recall/auto-capture as
kernel-owned lifecycle hooks; per-call character/token budgets; isolation pushed into the query.
**Reject:** the tool triple as the *whole* contract — OpenHuman's surface is an order of magnitude
larger (trees, diffs, goals, sources, sync, entities), which is exactly why capabilities must be
negotiated rather than assumed. **Also reject:** provider-authored lifecycle hooks. In OpenClaw the
plugin hooks the turn; here the kernel does, so policy cannot be bypassed by a driver.

---

## 3. The contract: `tinycortex-api`

### 3.1 Crate carve-out (the enabling move)

Today the shared value types live in `tinycortex::memory`, so any driver depending on the contract
drags in SQLite, the retrieval engine, and the job model. Split a **dependency-free** crate:

```
vendor/tinycortex/
├── api/          # NEW crate `tinycortex-api` — serde + std + async-trait only
│   └── src/      # value types, capability traits, Capabilities, MemoryError, CONTRACT_VERSION
└── src/          # the engine; depends on `tinycortex-api`, re-exports it as `tinycortex::memory`
```

This is the `skills`-gate type carve-out rule (`AGENTS.md`: inert types stay ungated, stub only
behaviour) applied one level up. Existing `tinycortex::memory::{…}` paths keep resolving via
re-export, so the ~30 host consumers and `memory::traits` are untouched.

Value types moving to the API crate verbatim: `MemoryEntry`, `MemoryCategory`, `MemoryTaint`,
`RecallOpts`, `NamespaceSummary`, plus chunk/source/tree DTOs (`ChunkRef`, `SourceRef`,
`TreeNodeRef`, `IngestRequest`, `IngestOutcome`, `DiffEntry`, `GoalRecord`, `ToolMemoryRecord`).

> `MemoryTaint` is security-critical and fails closed to `ExternalSync`. It moves **byte-identical**
> and keeps its dedicated seam test. Provenance semantics are contract, not implementation.

### 3.2 Capability families

A driver implements `MemoryProvider` plus any subset of the families it advertises:

| Family | Trait | Methods (indicative) | Required? |
| --- | --- | --- | --- |
| `core` | `MemoryCore` | `store`, `store_with_taint`, `get`, `forget`, `list`, `namespaces` | **yes** |
| `recall` | `MemoryRecall` | `recall(query, RecallOpts) -> Ranked<ChunkRef>` | **yes** |
| `ingest` | `MemoryIngest` | `ingest_document`, `ingest_chat` (driver owns chunking + embedding) | no |
| `documents` | `MemoryDocuments` | namespace-document tier: `put_doc`, `get_doc`, `query_docs` | no |
| `tree` | `MemoryTree` | `query_source`, `drill_down`, `seal`, `cascade` | no |
| `entities` | `MemoryEntities` | entity index + edges + hotness | no |
| `graph` | `MemoryGraph` | kv-graph read/write | no |
| `diff` | `MemoryDiff` | snapshot capture + change computation | no |
| `goals` | `MemoryGoals` | goal extraction/records | no |
| `tool_memory` | `MemoryToolMemory` | per-tool learned memory | no |
| `sources` | `MemorySourceSink` | accept synced source items (host owns creds + schedule) | no |
| `maintenance` | `MemoryMaintenance` | reembed, compact, consolidate ("dream"), doctor | no |
| `portability` | `MemoryPortability` | `export(stream)`, `import(stream)` | **yes** |

`core`, `recall`, and `portability` are mandatory: without them a driver is not a memory backend,
and without `portability` a user cannot leave it. Everything else degrades per kernel spec §3.3 —
the method is unregistered, the tool is absent, the UI hides the surface.

### 3.3 Degradation map (what a minimal driver loses)

| Absent capability | RPC unregistered | Agent tools absent |
| --- | --- | --- |
| `tree` | `memory_tree*`, retrieval drill-down | tree query/drill-down tools |
| `diff` | `memory_diff*` | diff tools |
| `goals` | `memory_goals*` | goal tools |
| `documents` | doc put/get/query | doc tools |
| `sources` | `memory_sources_sync`, `memory_sync*` | sync tools |

The registration sites are already grouped per family in `src/core/all.rs` (each
`all_memory_*_registered_controllers()` call), so this is a filter at those call sites — not a
rewrite. Both-ways tests per family, mirroring `channels_controllers_{registered,absent}`.

### 3.4 The guard (non-negotiable)

`MemoryGuard<P: MemoryProvider>` wraps the bound driver and is the only handle product code ever
receives. It enforces, in order:

1. `SecurityPolicy` tier + workspace/action-root path rules;
2. `source_scope` per-turn allowlist — **applied as a query predicate passed to the driver**, not
   as a post-filter (OpenClaw's isolation lesson; also what today's W5 seam test pins);
3. `MemoryTaint` stamping on every write — the driver receives taint, never assigns it;
4. redaction (`memory/util/redact.rs`) on content leaving the process for an `external` driver;
5. egress budget + `trust_state` check for `external` drivers;
6. char/token budgets for auto-recall/auto-capture;
7. audit event on the bus + tracing span with `driver_id`, `capability`, `namespace`.

Steps 4–5 are new and exist because "memory" is the most sensitive data in the product. An
`external` driver bind requires an explicit `trust_state = "trusted"` and, on first bind, a
one-time user consent recorded in config. Fail-closed: unset trust ⇒ refuse to bind, fall back to
embedded, surface in status.

### 3.5 Lifecycle hooks (kernel-owned)

- **auto-recall** — before an interactive turn, the kernel calls `recall` and injects results
  under a `max_context_tokens` budget (default 2000, OpenClaw parity).
- **auto-capture** — after a turn, the kernel decides *whether* to capture (existing
  `remember.rs` / `preferences.rs` policy) and calls `store`.
- **maintenance tick** — the existing scheduler drives `MemoryMaintenance::consolidate` if
  advertised; the embedded driver maps it to seal/cascade/reembed.

Drivers do not hook the agent loop. Same rule as `queue::run_once`: the host owns the loop, the
engine owns one step.

---

## 4. Drivers

### 4.1 `tinycortex` — embedded default

`src/openhuman/memory_adapter/embedded/` implements every family over the existing seam
(`src/openhuman/tinycortex/`). Zero new engine logic: it is a re-shaping of the current direct
calls into contract methods. Advertises all 13 families. This is the compatibility anchor — the
parity harness compares it against pre-change behaviour.

### 4.2 `http` — the external transport adapter

`src/openhuman/memory_adapter/http/` implements every family by translating to a documented
JSON wire contract, so a third-party backend never depends on Rust or on this repo:

```
POST /v1/handshake        → { contract_version, driver_id, capabilities[] }
POST /v1/memory/store     { namespace, key, content, category, taint, session_id }
POST /v1/memory/recall    { query, namespace, limit, filters, scope_allowlist[] } → ranked[]
POST /v1/memory/forget    { namespace, key | query }
POST /v1/memory/ingest    { source_ref, content, mime, taint }
GET  /v1/memory/export    → NDJSON stream
POST /v1/memory/import    ← NDJSON stream
GET  /v1/health           → { status, detail }
```

Unsupported family ⇒ the endpoint is absent from `capabilities[]` and returns `501`; the adapter
maps that to `MemoryError::Unsupported`. Auth via a keychain-resolved bearer. The handshake pins
the contract version; a major mismatch refuses the bind.

**`supermemory` reference driver** is a thin config profile over `http` (base URL, auth, field
mapping), shipped as a worked example plus a conformance-suite run — not special-cased in code.

### 4.3 `mcp` — opportunistic

For backends that already speak MCP, an adapter maps the families onto MCP tool calls through the
existing `mcp_client`. Lower priority; `http` covers the demand.

### 4.4 `composite` / `mirror`

Per kernel spec §3.5. `mirror` is the **migration path**: bind
`mirror { primary = "tinycortex", secondary = "supermemory" }`, backfill via
`export`→`import`, verify with the conformance suite, then re-bind to the secondary.

### 4.5 Config

```toml
[subsystems.memory]
driver = "tinycortex"

[subsystems.memory.hooks]
auto_recall = true
auto_capture = true
max_context_tokens = 2000
recall_max_chars = 1000
capture_max_chars = 500

[subsystems.memory.drivers.supermemory]
class = "external"; transport = "http"
endpoint = "https://api.supermemory.ai"
credential_ref = "keychain:supermemory"
trust_state = "untrusted"      # must be explicitly raised before bind succeeds
```

The existing `[memory]`, `[memory_tree]`, `[[memory_sources]]` blocks stay as-is and map into the
embedded driver's options; no user-visible config break.

---

## 5. RPC & tools (unchanged surface)

Method names, params, and payloads are **unchanged** — `memory*`, `memory_tree*`, `memory_sync*`,
`memory_sources*`, `memory_diff*`, `memory_goals*` all keep their contracts. Handlers stop calling
`memory::global::client()` and call `memory::subsystem::guard()` instead. Two additions:

- `memory_provider_status` — bound driver id, class, health, contract version, capabilities,
  last error. Drives the UI's capability-aware rendering.
- `memory_export` / `memory_import` — provider-agnostic NDJSON portability, gated by the approval
  gate (a full memory export is a high-consequence action).

---

## 6. Consolidation: what moves into TinyCortex

### 6.1 Re-disposition vs. the 2026-07-28 cutover evaluation

That evaluation asked *"is this product policy or engine logic?"* and concluded the remaining
`memory*` modules must stay. Under the kernel criterion (kernel spec §4) the question becomes
*"would a Supermemory-only build still need this file?"* — and several modules it retained are
**implementation of the default driver**, which is exactly where they belong once a second driver
is possible. Its core warning still holds and is honoured here: RPC, policy, secrets, and runtime
composition must **not** move into the crate.

### 6.2 Moves to `vendor/tinycortex`

| Host module | Why it moves | Lands as |
| --- | --- | --- |
| `memory_store/namespace_store/*` (the ten-table tier: `memory_docs`, `graph_*`, `episodic_log`+fts, `event_log`+fts+embeddings, `conversation_segments`, `segment_embeddings`, `vector_chunks`, `user_profile`) | TinyCortex-specific SQLite schema + migrations. Supermemory has no `episodic_fts`. | `store::namespace` behind the `documents`/`graph` capabilities |
| `memory_store/content/{wiki_git,obsidian,obsidian_registry}` | On-disk content formats of the embedded engine | `store::content::{wiki_git,obsidian}`, feature-gated |
| `memory_store/{client,factories,kinds,traits}.rs` compatibility shims | Re-exports over crate types; the contract replaces them | deleted |
| `memory_tree/health/`, `memory_tree/io.rs`, `summarise.rs` residue | Health/doctor of *this* engine | `tree::health`, surfaced via `MemoryMaintenance::doctor` |
| `memory_search/*` remaining shims | Re-export layer over crate retrieval | deleted |
| `memory_queue/{store,worker,scheduler,types}.rs` residue | Engine job model; host keeps only the tokio loop that calls `queue::run_once` | crate `queue` |
| `memory_sources/{readers,registry,reconcile,status}.rs` | Reader/parser implementations | crate `sources` |
| `memory_sync/{canonicalize,sources,workspace,composio}` engine parts not yet flipped | Sync engine; host keeps schedulers, creds, bus, RPC | crate `sync` (feature-gated network) |
| `memory_diff`, `memory_goals`, `memory_conversations`, `memory_tools` store/type re-export files | Thin facades over crate modules | deleted; import `tinycortex::memory::*` directly |
| `memory/{ingest_pipeline,tree_source,query,util/*}` engine internals | Chunking/ranking/tree policy mechanics of the embedded engine | crate `ingest`/`tree`/`retrieval` |

### 6.3 Stays in the host (kernel side)

`memory/{ops,schemas,schema,read_rpc,rpc_models}` · `memory/tools/*`, `memory_search/tools/`,
`memory_tools` tool surface · `SecurityPolicy` gating, `source_scope`, `util/redact.rs` ·
`preferences.rs`, `remember.rs`, `tree_policy.rs` (product policy over the tree, not the tree) ·
`global.rs` → becomes the registry/bind site · `chat.rs` · credentials/keychain, Composio OAuth ·
schedulers (`memory_sync/periodic.rs`), bus subscribers · config mapping · **new**
`memory_adapter/` (embedded, http, mcp, composite, guard).

### 6.4 How much actually moves (measured, 2026-07-28)

Measured over `src/openhuman/memory*`, classifying RPC/schema/ops/tools/bus/policy files as
kernel-side and everything else as engine:

| Module | Total | Kernel-side | Engine (movable) | Tests |
| --- | ---: | ---: | ---: | ---: |
| `memory_store` | 17,433 | 572 | **16,861** | 5,033 |
| `memory_sync` | 17,204 | 2,246 | **14,958** | 1,813 |
| `memory` | 18,332 | 12,474 | 5,858 | 2,709 |
| `memory_tree` | 9,898 | 3,549 | 6,349 | 292 |
| `memory_sources` | 4,968 | 1,733 | 3,235 | — |
| `memory_queue` | 1,411 | 39 | **1,372** | 37 |
| `memory_diff` | 1,956 | 1,695 | 261 | — |
| `memory_tools` | 1,338 | 461 | 877 | 228 |
| `memory_goals` | 869 | 642 | 227 | — |
| `memory_conversations` | 858 | 833 | 25 | — |
| `memory_search` | 718 | 703 | 15 | — |
| **Total** | **74,985** | **24,947 (33%)** | **50,038 (67%)** | 10,112 |

So roughly **two thirds of the host memory tree is movable engine code**, concentrated in
`memory_store`, `memory_sync`, `memory_tree`, and `memory_queue` — those four are 90% of the
movable mass and are near-totally engine (`memory_queue` is 97% engine, `memory_store` 97%).
Conversely `memory_conversations`, `memory_search`, `memory_diff`, and `memory_goals` are already
almost pure kernel-side surface: their remaining engine content is 25–261 LOC of facade, so those
directories effectively **collapse into shims** rather than "move".

This is the answer to "can most of it move now": **yes by mass, and mostly in four directories** —
subject to §6.5.

### 6.5 The one real blocker: engine→host reach-backs

Twelve movable files reach *back* into host state, so they cannot be lifted as-is. Inventory:

**(a) Task-local `source_scope` read from inside retrieval — the security-relevant one.**
`memory_tree/retrieval/{source,fast,drill_down,fetch,cover}.rs` call
`memory::source_scope::current_source_scope()` / `chunk_source_allowed_in()` directly. The
per-turn allowlist is a *host task-local* consumed *inside the engine*. Moving these files as-is
either drags `source_scope` into the crate (policy in the engine — the failure mode the
2026-07-28 cutover evaluation correctly warned about) or silently drops the allowlist.
**Fix:** invert it — retrieval takes an explicit `scope: Option<&ScopePredicate>` parameter, and
`MemoryGuard` populates it from the task-local at the call boundary. This is already the spec's
stated design (§3.4 step 2: *applied as a query predicate passed to the driver, not as a
post-filter*); it just has to land **before** the move, not after. Mechanical: five call sites,
one signature.
*(`memory_store/tools/raw_chunks.rs`, `memory_search/tools/{chunk_context,vector_search}.rs` also
read the task-local, but those are agent tools — kernel-side, staying put. No change needed.)*

**(b) Config/event-bus reach-backs from `memory_tree`.** ~20 files under `memory_tree/{tree,score,
graph,health,retrieval}` import `crate::openhuman::config::` or `crate::core::event_bus`.
**Fix:** pass the derived options in (the `MemoryConfig` mapping the seam's `config.rs` already
does) and emit through the existing sink traits instead of the bus directly. Mechanical, but
broader than (a) — this is the bulk of `memory_tree`'s move cost.

**Already fine, no work needed:** `memory_store/safety/*` is already a thin shim over the crate
scrubber; `namespace_store/query.rs` only *constructs* `MemoryTaint::Internal` (a type, which M0
puts in `tinycortex-api`); `memory_sources/readers/*` only mention redaction in log comments.

**Consequence for sequencing:** the M4-before-M8 rule is sharper than "all policy first". Only
class (a) is a policy-correctness gate. Class (b) is a decoupling chore that can run per-module in
parallel. So the move can start earlier and wider than the linear workstream table implies —
see the revised M8 split in §8.

### 6.6 The shims must stay host-owned

"Shims to expose the RPC APIs" has two possible shapes, and only one is correct:

- ✅ **Host-owned thin controllers over the contract.** `memory/schemas/*.rs` keeps defining the
  schema, keeps `handle_*` delegating — but delegates to `MemoryGuard<dyn MemoryProvider>` instead
  of a concrete TinyCortex path. The crate never learns that JSON-RPC exists.
- ❌ **Crate-owned RPC exposed through a host shim.** If `tinycortex` gains schemas/handlers and
  the host merely re-exports them, the dependency boundary inverts: the reusable engine now knows
  OpenHuman's method names, error envelope, and policy vocabulary — and a second driver can no
  longer satisfy the same RPC surface, which defeats the whole point.

The 51 controller schemas under `memory/schemas/` therefore **do not move**, even though they are
thin. Thin is what a good syscall table looks like.

### 6.7 Expected shape after

~74k LOC of host `memory*` reduces to a kernel-side surface dominated by RPC + tools + policy +
adapters. **Line count is not the goal and not the success metric** — the metric is that
`grep -rn "tinycortex::" src/openhuman/ --include=*.rs` returns hits only under
`memory_adapter/embedded/` and `src/openhuman/tinycortex/`.

---

## 7. Conformance suite

The golden-workspace parity harness that backs the TinyCortex cutover is promoted to a
**driver conformance suite** — one test corpus, run against any driver:

- **Tier 1 (mandatory families):** store/get/forget/list/namespaces round-trips, recall ranking
  sanity, taint preservation, namespace isolation, export→import fidelity.
- **Tier 2 (per advertised family):** one scenario per family; skipped-with-reason when unadvertised.
- **Tier 3 (policy, driver-independent):** `source_scope` allowlist honoured; out-of-scope source
  never returned; redaction applied before an `external` driver sees content; `ExternalSync` taint
  stamped on synced ingest; credential never in `Debug`/error output.
- **Tier 4 (differential):** `mirror` mode runs the corpus against both drivers and diffs results —
  the acceptance gate for adding a new driver.

`tinycortex` must pass Tiers 1–4 with results identical to pre-change. The reference `supermemory`
profile must pass Tiers 1–3 for its advertised set.

---

## 8. Workstreams

Each ≈ one PR; the sandwich rule applies to crate-side changes (crate PR → `chore(vendor): bump
tinycortex` → host cutover PR, host tests in the same PR for the ≥80% diff-coverage gate).

| # | Workstream | Deliverable | Gate |
| --- | --- | --- | --- |
| **M0** | `tinycortex-api` carve-out | Dep-free crate; `tinycortex::memory` re-exports; host imports unchanged | Full suite green; `cargo tree` shows no SQLite under `tinycortex-api` |
| **M1** | Contract definition | 13 capability traits, `Capabilities`, `MemoryError`, `CONTRACT_VERSION` | Compiles; no host wiring yet |
| **M2** | Registry + bind | `subsystems.memory` config, `SubsystemRegistry`, bind at `CoreBuilder`, fallback + status | `memory_provider_status` returns `tinycortex`/all caps |
| **M3** | Embedded driver | `memory_adapter/embedded/` implements all families over the existing seam | Conformance Tiers 1–2 identical to pre-change |
| **M4** | `MemoryGuard` | Policy decorator; all product call sites re-pointed; direct-driver-call lint test | Conformance Tier 3; existing security seam tests green |
| **M5** | Capability degradation | Filter controller registration + tool assembly by capability set; both-ways tests per family | `null` driver ⇒ memory RPC unknown-method, tools absent, core boots |
| **M6** | HTTP adapter + wire contract | `memory_adapter/http/`, handshake, `501`→`Unsupported`, egress/trust/redaction path | Conformance Tiers 1–3 against a mock backend |
| **M7** | Portability | `memory_export`/`memory_import` NDJSON + approval gate; `mirror` driver | Export→import round-trip; Tier 4 differential |
| **M8a** | Reach-back inversion (§6.5) | `scope` predicate parameter through retrieval; config/bus reach-backs replaced by injected options + sink traits | Existing `source_scope` seam test green; no `crate::openhuman::` import remains in the movable set |
| **M8b** | Bulk consolidation | The §6.2 moves — `memory_store`, `memory_sync`, `memory_tree`, `memory_queue` first (90% of the mass), one module per PR | Parity green per move; the `grep` invariant in §6.7 holds |
| **M8c** | Facade collapse | `memory_conversations`, `memory_search`, `memory_diff`, `memory_goals` reduce to kernel-side surface; re-export files deleted | Import paths land on `tinycortex::memory::*` |
| **M9** | Reference driver + docs | `supermemory` profile, conformance report, `gitbooks/developing/architecture/memory.md`, AGENTS.md checklist | Tiers 1–3 green for advertised set |

M0–M5 land the abstraction with zero behaviour change. M6–M7 make a second backend possible.

**Revised sequencing (per §6.5).** The original "M8 must not start before M4" was too coarse.
Sharper rule:

- **M8a class (a) — the `source_scope` inversion — is the hard gate.** It must land before any
  `memory_tree/retrieval` file moves, or the per-turn allowlist is dropped or dragged into the
  crate. It is five call sites and one signature; it can land immediately, in parallel with M0/M1.
- **M8a class (b) — config/bus decoupling — is a chore, not a gate**, and can run per-module
  concurrently with M2–M5.
- **M8b can therefore start once M8a is done for that module**, without waiting for M6/M7.
  `memory_store` (16.9k) and `memory_queue` (1.4k) have almost no reach-backs and are movable
  first; `memory_tree` (6.3k) is gated on M8a(b); `memory_sync` (15.0k) is gated on its credential
  and scheduler seams staying host.
- **M4 (`MemoryGuard`) still gates M8c**, because the facade collapse is what re-points call sites
  off `memory::global::client()` — that is the moment enforcement either exists or doesn't.

---

## 9. Risks

| Risk | Mitigation |
| --- | --- |
| **Policy dropped during the move** (highest) | M4 before M8; Tier-3 conformance runs on every driver; guard is the only handle |
| **Memory exfiltration via an external driver** | fail-closed `trust_state`, one-time consent, redaction before egress, egress budget, audit events, `null`-fallback on bind failure |
| **Perf regression from trait indirection** | families are coarse; `async_trait` boxing on already-async I/O paths is noise; benchmark recall p50/p95 before/after in M3 |
| **Capability explosion** | 13 families capped; adding one = contract minor bump + both-ways test |
| **Crate split churn** | M0 is re-export-only; every existing import path keeps resolving |
| **Feature-forwarding drift** | any new default-ON gate goes into `app/src-tauri/Cargo.toml`; `check-feature-forwarding.mjs` enforces it |
| **Disabled-build test rot** | CI's smoke lane is `cargo check` only — run `cargo test --lib --no-default-features …` locally after every gated change |

---

## 10. Open questions

1. **Does `documents` (the namespace tier) stay mandatory in practice?** Several host surfaces
   (`store_skill_sync`, profiles, episodic log) assume it. If an external driver cannot provide it,
   do we bind a `composite` with an embedded `documents` shard, or degrade those surfaces?
   *Leaning:* composite — keep documents embedded, delegate recall/ingest. Decide in M5.
2. **Where do embeddings live for an external driver?** If the backend embeds server-side we must
   not double-embed. *Proposal:* a capability flag `embeds_internally`; when set, the kernel skips
   the embedding provider for that path.
3. **Multi-user / multi-workspace binding** — one bind per process, or per workspace? *Leaning:*
   per workspace, since `global.rs` already rebinds on active-user switch.
4. **Sync ownership.** Sources/credentials/scheduling stay host, but a backend like Supermemory
   has its own connectors. Do we allow a driver to advertise `owns_sources` and let the host step
   back? Deferred past M9.


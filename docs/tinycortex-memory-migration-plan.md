# TinyCortex Memory Migration — Plan & Audit

**Status:** draft plan — no code changes yet.
**Anchor precedent:** the TinyAgents harness migration (#4249 / #4399 / #4473 and follow-ups).
**Target:** migrate large portions of the OpenHuman memory subsystem onto the `tinycortex` crate, vendored as a git submodule at **`vendor/tinycortex`** (`https://github.com/tinyhumansai/tinycortex`).

---

## 0. Ground truth (as audited)

### 0.1 What exists today

**Host memory subsystem** (in-tree, all still live):

| Host module | LOC (approx) | Role |
| --- | --- | --- |
| `src/openhuman/memory/` | ~20,700 (79 files) | Orchestration/policy layer: `Memory` trait, RPC ops/schemas, agent tools, ingest orchestrator, sync lifecycle, preferences, source-scope task-locals, redaction |
| `src/openhuman/memory_store/` | ~32,800 | Storage: `UnifiedMemory`/`MemoryClient`, chunks DB, content store, vectors, KV, safety |
| `src/openhuman/memory_tree/` | ~25,300 | Summary trees, retrieval, scoring, health |
| `src/openhuman/memory_sync/` | ~30,700 | Live sync: Composio, MCP, OAuth, pollers |
| `memory_queue`, `memory_sources`, `memory_diff`, `memory_goals`, `memory_entities`, `memory_graph`, `memory_archivist`, `memory_tools`, `memory_search` | ~20k combined | Engine long tail |

**TinyCortex** (`vendor/tinycortex`, crate `tinycortex` v0.1.1, MIT, single crate, ~49,500 lines under `src/memory/`): an **already-completed port** of the engine layers — `store/` (content markdown + YAML, SQLite vectors, KV, entity_index, safety), `chunks/`, `tree/`, `queue/`, `retrieval/`, `score/`, `ingest/` (canonicalize/extract/pipeline), `conversations/`, `archivist/`, `diff/`, `sources/` (local readers only), `entities/`, `graph/`, `goals/`, `tool_memory/`. Its git history is explicitly "port X from OpenHuman" commits, and its types (`MemoryEntry`, `MemoryCategory`, `MemoryTaint`, `RecallOpts`, `NamespaceSummary`, the `Memory` trait) are wire-compatible with the host's `memory::traits`.

**Wiring already pre-staged:**
- `.gitmodules` has `vendor/tinycortex` → `tinyhumansai/tinycortex`.
- Root `Cargo.toml` `[patch.crates-io]` block includes `tinycortex = { path = "vendor/tinycortex" }` (same mechanism as tinyagents: crates.io version pin + path patch). `app/src-tauri/Cargo.toml` has a matching path entry.
- CI (`test-reusable.yml`, `build-ci-image.yml`, release workflows) already checks out submodules recursively. **No new CI plumbing needed.**

**Gap:** zero `.rs` files in `src/` import `tinycortex`. There is no `[dependencies] tinycortex = "0.1"` entry activating the patch. The host still runs entirely on the in-tree engine, which has continued to evolve since the port was taken.

### 0.2 TinyCortex's declared boundary (the seam contract)

TinyCortex deliberately does **not** own (per its own module docs):
- Live sync / polling / OAuth / network source readers (Composio, GitHub, RSS, web) — only `folder` and `conversation` local readers ship.
- RPC schemas, event bus, CLI, agent-tool surfaces.
- LLM and embedding **compute** — abstracted behind `EmbeddingBackend`, `score::extract::ChatProvider`, `tree::summarise::Summariser` traits ("never makes a network call").
- A tokio worker pool — the host drives the queue via `queue::run_once` / `drain_until_idle`.
- The namespace document/graph store and the Composio identity registry.

This boundary is the migration contract: **engine in TinyCortex, product policy + I/O + surfaces in OpenHuman** — exactly mirroring the tinyagents split (generic runtime in the crate; prompts, security, RPC compat, UX in the host).

### 0.3 Submodule contribution workflow (how engine changes are made)

TinyCortex lives at **`vendor/tinycortex`** as a git submodule. Any change to engine code — whether by a human or by an LLM agent — is made **inside the submodule**, committed on a branch there, and raised as a **PR against `tinyhumansai/tinycortex`**. Once merged, the host repo bumps the submodule pointer in a standalone `chore(vendor): bump tinycortex — <summary> (tinycortex#<PR>)` commit, keeping the `[dependencies]` version pin in lockstep (same convention as `chore(vendor): bump tinyagents` commits citing `tinyagents#11` etc.). Host-side PRs never contain engine source edits; they only move the submodule SHA.

---

## 1. Target ownership split

### Moves to TinyCortex (delete from host after cutover)

| Host module | TinyCortex counterpart | Notes |
| --- | --- | --- |
| `memory_store/` (chunks, content, vectors, kv, entity_index, safety) | `store/`, `chunks/` | Largest single win (~33k LOC) |
| `memory_tree/` (tree, retrieval, score, summarise) | `tree/`, `retrieval/`, `score/` | Host keeps `tree_policy.rs` flavour constants |
| `memory_queue/` | `queue/` | Host keeps the tokio worker loop that drives it |
| `memory/ingest_pipeline.rs` internals | `ingest/` | Host keeps the thin entry points (`ingest_chat`, `ingest_document_with_scope`) as adapters |
| `memory_diff/`, `memory_entities/`, `memory_graph/`, `memory_goals/`, `memory_archivist/`, `memory_sources/` (registry + local readers), `tool_memory` engine, `conversations` engine | same-named modules | Long tail |
| `memory/traits.rs` core types | `tinycortex::memory::{Memory, MemoryEntry, MemoryCategory, MemoryTaint, RecallOpts, NamespaceSummary, …}` | Host re-exports from the crate so 30+ consumer sites keep compiling unchanged |

### Stays in OpenHuman (product policy, I/O, surfaces)

- **All RPC surfaces**: `memory/ops/`, `memory/schemas/`, `memory/schema/`, `memory/read_rpc/`, `rpc_models.rs` (controller framework types `ControllerSchema`/`RpcOutcome` are host-only). JSON-RPC method names and payload shapes must not change.
- **Agent tools**: `memory/tools/` and `memory/query/` (`Tool`/`ToolResult` impls, `SecurityPolicy` gating) — they become thin wrappers over crate retrieval primitives.
- **Live sync**: all of `memory_sync/` (Composio/MCP/OAuth/pollers), `memory/sync.rs` lifecycle + event-bus stage events.
- **Process glue**: `memory/global.rs` singleton + background queue worker; `memory/source_scope.rs` tokio task-locals; `memory/chat.rs` (LLM adapter over `openhuman::inference`); embeddings provider wiring.
- **Policy/UX**: `preferences.rs`, `remember.rs`, `tree_policy.rs`, `util/redact.rs`, config mapping (`Config` → `tinycortex::MemoryConfig`).
- **Namespace document/graph store** (until/unless deliberately upstreamed — TinyCortex explicitly excludes it today).

### The adapter seam: `src/openhuman/tinycortex/`

New sibling module mirroring `src/openhuman/tinyagents/`, holding every impl of a TinyCortex trait over an OpenHuman service:

- `embeddings.rs` — `impl tinycortex EmbeddingBackend` over `openhuman::embeddings` (dim/model/signature from `Config`).
- `chat.rs` — `impl ChatProvider` + `impl Summariser` over `memory::chat::build_chat_provider` / `inference::provider`.
- `queue_driver.rs` — tokio worker loop calling `queue::run_once`/`drain_until_idle`, owned by `memory/global.rs`, with event-bus progress emission and Sentry hooks (host-side, since TinyCortex dropped its scheduler).
- `config.rs` — `Config` → `MemoryConfig` (workspace roots, `EmbeddingConfig`, `TreeConfig`, `WeightProfile`, `SyncBudgetConfig`) with `tree_policy.rs` flavour overlays.
- `sinks.rs` — `TreeJobSink`, `SnapshotItemSource` impls bridging to host state.
- `bus.rs` — translate engine outcomes into `core::event_bus` `DomainEvent`s (host-side only; the crate stays bus-free).
- `mod.rs` — facade re-exports (`pub use tinycortex::memory::{…}`) so the rest of the host imports through one seam, plus module-doc explaining the boundary (the tinyagents seam's `mod.rs` header is the template).

---

## 2. Phase 0 — Audit & baseline (before any cutover)

This phase produces documents and upstream issues only; it is the gate for everything after.

**0.1 Drift audit.** The port in TinyCortex was taken at a point-in-time; the host engine has since received changes (perf waves, bug fixes). For each host module → crate module pair in the table above, diff behavior and enumerate host commits since the port SHA that must be re-applied upstream. Output: a per-module drift ledger (`host commit → tinycortex PR needed / already present / obsolete`). This is the highest-risk unknown in the whole migration — **nothing cuts over until its module's drift ledger is closed.**

**0.2 API gap audit.** Enumerate every host call site into `memory_store`/`memory_tree`/`memory_queue`/long-tail modules (the internal sibling graph: ~25 uses of `memory_store::chunks`, 6 of `trees`, etc.) and map each to a TinyCortex public API. Output: gap list, each gap becoming a `tinycortex` issue/PR (e.g. the known gaps: graph edge accumulation at persist time, seal-time embedding, `seal_document_subtree` follow-ups, tree health/doctor — `memory/tools/doctor.rs` wraps `memory_tree::health`, which the crate defers).

**0.3 Data-format parity audit.** Existing user workspaces must open unchanged after cutover. Verify byte/schema compatibility for: chunks.db SQLite schema + migrations, jobs table, tree tables, vector table encoding (packed f32), markdown content store paths + YAML frontmatter, entity markdown, git diff-ledger layout, deterministic chunk IDs. Output: a parity checklist with a fixture-based verification harness design (golden workspace snapshot opened by both engines, compared). Any mismatch is an upstream fix, not a host workaround.

**0.4 Toolchain baseline.** Add `tinycortex = { version = "0.1" }` under `[dependencies]` (activating the existing `[patch.crates-io]` override); align `rusqlite` versions between host and crate (both must link one bundled SQLite); check edition (crate is 2021), feature flags, and that **both Cargo worlds** (root crate and `app/src-tauri`) compile with the dep active; confirm `GGML_NATIVE=OFF` macOS builds. Verify the release workflows' submodule-init covers `vendor/tinycortex` (the tinyagents wave needed +5-line fixes there).

**0.5 Type-unification decision.** Host `memory::traits` types and crate types are wire-compatible twins. Decide: host re-exports crate types (preferred — one source of truth, 30+ consumer sites unchanged via `pub use`), vs. keeping host types + `From` conversions (fallback if serde/API divergence is found in 0.3). Special care: `MemoryTaint` is **security-critical provenance** (fails closed to `ExternalSync`, drives external-effect-tool gating) — its semantics, serde representation, and fail-closed defaults must be proven identical before re-exporting.

**0.6 Spec doc.** Write `docs/tinycortex-migration-spec.md` version-anchored to exact reviewed SHAs (host + tinycortex), with the ownership lists above, the drift/gap/parity ledgers, and a **deletion ledger** skeleton (every legacy file, with preconditions for deletion) — directly modeled on `docs/tinyagents-migration-spec.md` + `99-deletion-ledger.md`.

---

## 3. Cutover workstreams (Phase 1–8)

Per the tinyagents rules: **adapter first → prove parity → flip ownership → delete legacy**, deletion mandatory and enumerated per step. Ordering follows the engine's dependency graph (storage first, surfaces last). Within every workstream, **implementation lands first, tests second** (see §5).

**W1 — Seam scaffolding.** Create `src/openhuman/tinycortex/` with the adapters in §1 (config, embeddings, chat/summariser, queue driver, sinks, bus bridge, facade). No behavior flips yet; adapters are constructed and unit-verified against the crate's inert defaults. *Deliverable: crate is linked, adapters compile, `MemoryConfig` derived from real `Config`.*

**W2 — Types & trait cutover.** `memory/traits.rs` becomes re-exports of `tinycortex` types (per 0.5 decision). All 30+ external consumers (`agent/harness`, `learning`, `channels/runtime`, `subconscious`, `threads`, …) compile unchanged through the re-export. `sqlite_conn()` escape hatch on the host trait is reviewed: either upstreamed or kept as a host-side extension trait.

**W3 — Store + chunks.** Flip `memory_store` internals (`UnifiedMemory`/`MemoryClient` re-implemented over `tinycortex::store` + `chunks`), keeping the host-facing `MemoryClient` API stable so `global.rs` and all RPC ops are untouched. Delete legacy `memory_store` engine files. *Gate: 0.3 parity harness green on a golden workspace.*

**W4 — Queue.** `memory_queue` → `tinycortex::queue`, driven by the seam's tokio worker. Job payload/schema parity from 0.3. Delete `memory_queue`.

**W5 — Tree + retrieval + score.** `memory_tree` → `tinycortex::{tree, retrieval, score}`. Host `memory/query/*` tools and `read_rpc/*` re-point to crate primitives (`query_source`, `drill_down`, `cover_window`, `fetch_leaves`, `search_entities`, MMR/hybrid scoring). `tree_source/registry.rs` wraps the crate registry; `tree_policy.rs` stays host. `source_scope` enforcement point re-verified (retrieval must still respect the per-turn allowlist — this is a security surface). Delete `memory_tree`.

**W6 — Ingest.** `memory/ingest_pipeline.rs` + `memory/ingestion/` re-pointed to `tinycortex::ingest` + `score::extract` (LLM extraction via the seam's `ChatProvider`). The namespace document/graph store path stays host-side unless deliberately upstreamed (explicit decision in this workstream). `ingest_chat`/`ingest_document_with_scope` keep their signatures — 11 call sites (learning, agent harness, archivist) unchanged.

**W7 — Long tail.** `memory_diff`→`diff`, `memory_entities`→`entities`, `memory_graph`→`graph`, `memory_goals`→`goals`, `memory_archivist`→`archivist`, `memory_sources` registry/local readers→`sources`, tool-memory engine→`tool_memory`, conversation storage→`conversations`. Each is small and independent; each deletes its legacy module on flip. `memory_sync` explicitly **does not move** — it keeps writing through the crate's ingest/source contracts.

**W8 — Test port + parity sweep + deletion-ledger close-out.** See §5. Ends with: deletion ledger fully executed, `gitbooks/developing/architecture.md` + a new `architecture/memory.md` seam doc written (the durable post-plan documentation, as `agent-harness.md` was for tinyagents), `AGENTS.md`/`CLAUDE.md` module tables updated, and the spec doc archived.

### Sequencing note

W3–W5 are the risky core (user data on disk). W1–W2 can land quickly; W6–W7 parallelize after W5. Expect the real-world shape to match tinyagents: one or two substantial cutover-wave PRs plus a long tail of small parity-fix PRs — budget for that tail explicitly.

---

## 4. Git / PR / submodule workflow

- **Branching:** all work on feature branches off `upstream/main` (never on `main`), small focused commits, explicit `git add <paths>` (never `-A`), one workstream ≈ one PR against `tinyhumansai/openhuman`.
- **Engine changes → submodule PRs:** any modification to engine code (drift re-application, gap-filling, parity fixes) is committed inside `vendor/tinycortex` on a branch and PR'd to `tinyhumansai/tinycortex`. This applies equally to human contributors and LLM agents: *change the submodule, raise the PR from there.* Host PRs consume merged engine changes via `chore(vendor): bump tinycortex — <what> (tinycortex#<n>)` commits.
- **Version lockstep:** keep the `[dependencies] tinycortex = "<version>"` pin in lockstep with the submodule tag; publish tinycortex to crates.io at each milestone so non-vendored consumers resolve (identical to the tinyagents `1.5.0` + patch pattern).
- **Interleaving:** a typical workstream is a sandwich — (a) tinycortex PR(s) closing that module's drift/gap ledger, (b) host `chore(vendor)` bump, (c) host cutover PR (adapter flip + legacy deletion), (d) host test PR/commits. CI already handles recursive submodule checkout on all build/test lanes.

---

## 5. Testing strategy — implementations first, tests second

Ordering rule for this migration: **within each workstream, the implementation (adapter + cutover + deletion) lands first; the test work follows as the second slice.** Tests are not skipped — they are sequenced after the implementation is proven to compile and pass the *existing* suites. Concretely:

1. **Slice A (impl):** adapters + cutover + legacy deletion. Gate: `cargo check` both worlds, existing crate-level integration tests (`tests/memory_roundtrip_e2e.rs`, `memory_tree_sync_deep_raw_coverage_e2e.rs`, etc.) still green — these exercise the public `openhuman::memory::` surface and act as the built-in parity harness for every flip.
2. **Slice B (tests):** port and extend the test estate for the new boundary:
   - **Engine-internal tests move upstream.** The host's `#[path]`-included sibling tests (`ops_tests.rs` pattern) that poke engine internals migrate into `vendor/tinycortex` as crate tests (via tinycortex PRs), following the crate's own `*_tests.rs` sibling convention.
   - **Host keeps boundary tests:** RPC schema/handler tests, tool gating tests, seam adapter tests (config mapping, embedding signature, taint fail-closed, source-scope enforcement), and the E2E files under `tests/`.
   - **Test-harness re-plumbing** is its own task: `GLOBAL_MEMORY_TEST_LOCK`, `ensure_shared_memory_client`, and `config::TEST_ENV_LOCK` are host globals the sibling tests depend on; upstreamed tests need crate-local fixtures instead.
3. **Coverage gate reality check:** PR CI enforces ≥80% diff coverage on changed lines, and host coverage tooling does not count tests living in the vendored crate. So "tests second" means *second slice of the same PR* (impl commits, then test commits, one PR) for host-side changes — not a separate later PR — or the coverage gate blocks the merge. Pure-deletion diffs and the seam re-export shims are the low-coverage-risk parts; the seam adapters need their host-side unit tests in the same PR.
4. **W8 parity sweep (last):** golden-workspace fixture opened by pre- and post-migration builds (read-side comparison of recall/retrieval/tree output), full `pnpm test:rust` + JSON-RPC E2E + the memory-related `tests/*_e2e.rs` matrix, plus a manual upgrade test on a real developer workspace.

---

## 6. Risk register

| Risk | Severity | Mitigation |
| --- | --- | --- |
| **Engine drift** since the tinycortex port (host perf/bug fixes not upstream) | High | Phase 0.1 drift ledger is a hard gate per module; no cutover with an open ledger |
| **On-disk format divergence** (SQLite schemas, chunk IDs, vault layout) breaking existing user workspaces | High | Phase 0.3 golden-workspace parity harness; upstream fixes only, never host shims |
| **`MemoryTaint` / `source_scope` security semantics** silently weakened across the boundary | High | Dedicated seam tests for fail-closed taint and scope enforcement; treated as security review items in W2/W5 |
| **Blast radius** of `memory::traits` (30+ sites), `global` (25), `chat` (~20), `redact` (12) | Medium | Re-export strategy keeps import paths stable; `redact`/`chat`/`global` never move |
| **rusqlite / dependency version skew** (two bundled SQLites, two Cargo worlds) | Medium | Phase 0.4 alignment before any dep activation; watch `app/src-tauri` lockfile too |
| **Coverage gate vs. vendored tests** (host CI can't count crate-side tests) | Medium | Host-side seam/boundary tests in the same PR as impl (§5.3) |
| **Sibling `#[path]` tests bound to private internals** won't move cleanly | Medium | Explicit test-port slice per workstream; crate-local fixtures replace host globals |
| **Queue driver behavior change** (crate has no scheduler; host loop replaces tokio pool + Sentry hooks) | Medium | W4 keeps worker cadence/backoff/error-reporting parity; verbose `[memory]`-prefixed logging per repo logging rules |
| **Namespace doc/graph store ambiguity** (crate excludes it) | Low | Explicit keep-host decision in W6; revisit upstreaming later |
| **Long parity tail** (tinyagents needed ~15 follow-up PRs) | Expected | Budget the tail; isolate each parity fix as a small scoped PR, engine fixes via submodule PRs |

---

## 7. Definition of done

- `memory_store`, `memory_tree`, `memory_queue`, `memory_diff`, `memory_entities`, `memory_graph`, `memory_goals`, `memory_archivist`, engine parts of `memory_sources`/`memory_tools`/conversations deleted from `src/openhuman/`; deletion ledger fully checked off.
- Host memory code = policy/surfaces only: `memory/` (RPC, tools, sync lifecycle, preferences, scope, redact, global) + `src/openhuman/tinycortex/` seam + `memory_sync/`.
- All engine logic served by `tinycortex` at a tagged, crates.io-published version, submodule pinned in lockstep.
- JSON-RPC method names/payloads unchanged; existing user workspaces open and recall identically (golden-workspace parity green).
- Full suites green on both CI lanes; gitbooks/AGENTS.md updated; spec + ledgers archived.

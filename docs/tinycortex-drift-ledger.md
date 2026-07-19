# TinyCortex Drift Ledger (Phase 0.1)

**Purpose.** The `tinycortex` port was taken at a point in time; the OpenHuman host
engine has continued to evolve since. This ledger enumerates every host commit that
touched an engine-mapping memory module after the port line, and classifies each as:

- **DRIFT → tinycortex PR** — a real engine behavior change absent from the crate; must be
  re-applied upstream (submodule PR against `tinyhumansai/tinycortex`) before that module cuts over.
- **HOST-OWNED** — the change lives in a layer that stays in OpenHuman (RPC, agent tools,
  event bus, embedding *compute*, live sync). No upstream needed.
- **HOST-RETAINED (crate excludes)** — an engine-adjacent feature the crate *deliberately*
  does not own (declared in its own module docs). Stays host; may imply a **seam gap** (see
  the API gap audit).
- **ALREADY PRESENT** — the change is already in the crate (port captured it).

> **Gate rule (plan §2/§6):** no module cuts over while its drift ledger row is open.
> A row is *closed* when its DRIFT items are merged upstream and the submodule SHA is bumped,
> or when the item is reclassified HOST-OWNED / HOST-RETAINED / ALREADY PRESENT.

## Anchors

| Thing | Value |
| --- | --- |
| Host repo | `tinyhumansai/openhuman` |
| Host audit SHA | `7850cf363559bcbb7ba688cbc4fccdb6bd9ce754` (`main`, 2026-07-04) |
| TinyCortex submodule | `vendor/tinycortex` → `tinyhumansai/tinycortex` |
| TinyCortex audit SHA | `d1a8c7be2babc8fff7a72ed93861f459f3d6fa58` |
| **TinyCortex pinned SHA (current)** | `a8e10f7dd8ebdb9b0905e1380fefcc6bf5a65207` — audit SHA **+ #59** (native-dep alignment, §0.4) **+ #63/#64** (D2/D1 drift ports) merged; this is the live gitlink |
| TinyCortex crate version | `0.1.1` |
| **Port line (derived)** | **after 2026-06-25, before 2026-06-28** (see below) |

> **Execution status (2026-07-10). Drift closure COMPLETE — all rows CLOSED.**
> The submodule is pinned at `a8e10f7`. **D3 CLOSED** (folded into #59);
> **D2 CLOSED** — merged as **tinycortex#63** (`is_host_io_error`, `e352435`);
> **D1 CLOSED** — merged as **tinycortex#64** (rank-before-materialize, `a8e10f7`).
> Host gitlink bumped `33dda94 → a8e10f7` (`chore(vendor): bump tinycortex …
> (tinycortex#63, #64)`). With every drift row closed, **W4** (queue) and **W7**
> (conversations) are now unblocked per the gate rule. D4 (memory_sync corpus,
> W-SYNC) remains its own separate track.

### How the port line was located

The port commits in `vendor/tinycortex` are all dated **2026-06-29**, but that is the date the
*port* was authored, not the host state it captured. The line was pinned by **content**, not date:

- **≥ 2026-06-25 is captured.** Host `feat(memory_diff): back change ledger with git instead of
  SQLite` (`040e6e20d`, 06-25) replaced the SQLite `mem_diff_read_markers` table with a git-backed
  ledger. The crate's `diff/` uses the **git-backed** `ledger.get_read_marker(...)`
  (`vendor/tinycortex/src/memory/diff/diff.rs:98`, `ledger.rs`) — i.e. the post-06-25 shape. So the
  port base includes the 06-25 memory_diff work.
- **< 2026-06-28 for engine features.** Host `feat(memory): track summary-only wiki git history`
  (`6395f642e`, 06-28) added `memory_store/content/wiki_git/`. The crate has **no** `wiki_git` file
  anywhere — but see the reclassification below: the crate *deliberately* excludes it, so this is not
  proof of a stale base, it is a declared boundary.

Net: only commits after 2026-06-25 that touch engine-mapping modules are drift candidates, and each
was verified against crate content individually below.

---

## Drift candidates (verified individually against crate content)

Scan: `git log --since=2026-06-20 -- src/openhuman/memory_store memory_tree memory_queue memory_diff
memory_goals memory_entities memory_graph memory_archivist memory_conversations memory_sources`,
then per-commit file lists intersected with engine-mapping modules, then content-diffed against
`vendor/tinycortex`.

### DRIFT → needs tinycortex PR

| # | Host commit | Module | Change | Crate state (verified) | Upstream target |
| --- | --- | --- | --- | --- | --- |
| D1 | `007a99b62` (06-30) `perf(memory_conversations): rank before cloning hits in cross-thread search` | `memory_conversations/inverted_index.rs` | Rank matches on cheap borrowed keys (`(doc_id:u32, matched:usize, created_at:&str)`), truncate to `limit`, **then** materialize the KB-sized `CrossThreadHit`. Order-equivalent to score ranking. | ✅ **CLOSED.** Ported + merged as **tinycortex#64** (`a8e10f7`): the rank-before-materialize refactor + `ranks_by_score_then_recency_before_truncating` test. Was pre-fix clone-then-rank at the port line. | — (closed) |
| D2 | `d7bee77e3` (06-30) `fix(memory-queue): classify host-FS I/O errors to stop the tree_jobs Sentry flood` | `memory_queue/worker.rs` | Adds `is_host_io_error(&anyhow::Error) -> bool` classifying **persistent** host-FS failures (EIO/ENOSPC/EROFS) distinct from transient SQLite busy/I-O, so the worker backs off and reports Sentry **once** instead of ~10k events/50min (Sentry CORE-RUST-19J). | ✅ **CLOSED.** Ported + merged as **tinycortex#63** (`e352435`): the `is_host_io_error` predicate + 2 unit tests (EIO/ENOSPC/EROFS × typed/context/text; negatives). **Predicate only** — the Sentry-once emission and `mark_storage_degraded` flag stay host-owned (see D2-host below). | — (closed) |
| D3 | `c43f79641` (07-03) (within TinyAgents migration) | `memory_store/vectors/store.rs` | `count()` reads `COUNT(*)` as `i64` and converts via `usize::try_from(...).context(...)` instead of `row.get::<usize>` directly — robustness against platform `usize`/`i64` mismatch. | ✅ **CLOSED.** Present at `vendor/tinycortex/src/memory/store/vectors/store.rs:371–384` (`usize::try_from(count).context(...)`) — folded into #59's `usize`→`i64`/`try_from` rusqlite-0.40 sweep and merged. | — (closed) |

**Drift rows: D1, D2 (predicate), D3** — the only three engine behavior changes since the port line,
all small and independent, **now all CLOSED** (gitlink `a8e10f7`).

- D1 gates **W7** (long tail — conversations). ✅ **CLOSED** — merged as tinycortex#64.
- D2 gates **W4** (queue). ✅ **CLOSED** — merged as tinycortex#63.
- D3 gates **W3** (store + chunks). ✅ **CLOSED** — already in the crate via #59.

### HOST-OWNED — same commits, layers that stay in OpenHuman (no upstream)

| Host commit | File(s) | Layer | Why host |
| --- | --- | --- | --- |
| `0304d145f` (07-03) | `memory/tools/store.rs`, `memory/tools/forget.rs` | Agent tools | Tool contract/prompt text; agent tools stay host (plan §1). |
| `7bf18562a` (06-30) | `memory/read_rpc/{types,vault}.rs` | RPC read surface | `read_rpc` stays host; JSON-RPC surface. |
| `f84eec533` (06-30) | `memory_conversations/bus.rs` | Event bus | `bus.rs` = `EventHandler` impls, host-owned by canonical module shape. |
| `6edaa77b1` (06-29) | `memory_tree/score/embed/openai_compat.rs` | Embedding **compute** | Network-calling embedding backend; the crate abstracts compute behind `EmbeddingBackend` and "never makes a network call". Wires into the W1 `embeddings.rs` seam. |
| `d7bee77e3` (06-30) [D2-host] | `memory_tree/health/{mod,doctor}.rs` (`mark_storage_degraded`/`clear_storage_degraded`), `memory_tree/tree/rpc.rs` | Health signal + RPC | Degraded-state flag + Sentry wiring + doctor RPC. Crate defers tree health entirely (see gap audit); this is the host-side consumer of D2's predicate. |
| `c43f79641` (07-03) | `memory_search/{vector,tools}/*`, `memory_sync/composio/*` | Agent tools / live sync | Import-path churn from the TinyAgents cutover + live-sync; not engine semantics. |

### HOST-RETAINED — crate deliberately excludes (not drift)

| Host commit | File(s) | Crate declaration |
| --- | --- | --- |
| `6395f642e` (06-28) `feat(memory): track summary-only wiki git history` | `memory_store/content/wiki_git/` (mod + tests, ~690 LOC), plus a seal-time hook in `memory_tree/ingest.rs` + `memory_tree/tree/bucket_seal.rs` | `vendor/tinycortex/src/memory/store/content/mod.rs:19–20`: *"The Obsidian-vault registry (`content::obsidian*`) and the git-backed wiki mirror (`content::wiki_git`) pull host config and git surfaces beyond this."* The crate explicitly leaves `wiki_git` **and** `obsidian*`/`obsidian_registry` host-side (host `memory_store/content/mod.rs:17,18,23`). |

**Reclassification note (important).** At first pass this looked like drift (feature absent from crate).
It is **not** — the crate's own content module doc names `content::wiki_git` and `content::obsidian*` as
host surfaces it does not own. So `wiki_git`, `obsidian`, `obsidian_registry` join `memory_sync` as
**host-retained** parts of an otherwise-moving module. **Consequence:** the seal-time hook that
`6395f642e` wired into `bucket_seal.rs` has **no counterpart callback in the crate's `bucket_seal`**
(`vendor/tinycortex/src/memory/tree/bucket_seal.rs` exposes no post-seal sink). That is tracked as an
**API gap** (a `TreeJobSink`-style "summary sealed" callback the host implements to drive `wiki_git`),
not as drift. See `tinycortex-api-gap-audit.md`.

---

## Per-module drift status (the gate table)

| Engine module | Maps to crate | Open drift | Gates workstream | Status |
| --- | --- | --- | --- | --- |
| `memory_store` (chunks, content, vectors, kv, entity_index, safety) | `store/`, `chunks/` | **D3** (vectors count guard) ✅ closed via #59. `wiki_git`/`obsidian*` host-retained (not drift). | W3 | **CLEAR** (D3 closed) |
| `memory_tree` (tree, retrieval, score, summarise) | `tree/`, `retrieval/`, `score/` | none (health/rpc/embed-compute are host-owned) | W5 | **CLEAR** |
| `memory_queue` | `queue/` | **D2** (predicate) — ✅ merged tinycortex#63 | W4 | **CLEAR** (D2 closed) |
| `memory_conversations` | `conversations/` | **D1** (rank-before-clone) — ✅ merged tinycortex#64 | W7 | **CLEAR** (D1 closed) |
| `memory_diff` | `diff/` | none (git-ledger captured) | W7 | **CLEAR** |
| `memory_entities` | `entities/` | none | W7 | **CLEAR** |
| `memory_graph` | `graph/` | none | W7 | **CLEAR** |
| `memory_goals` | `goals/` | none | W7 | **CLEAR** |
| `memory_archivist` | `archivist/` | none | W7 | **CLEAR** |
| `memory_sources` (registry + local readers) | `sources/` | none | W7 | **CLEAR** |
| `memory_tools` (engine part) | `tool_memory/` | none | W7 | **CLEAR** |
| `memory_search` (`vector`, `scoring` engine parts; `tools` are host) | `retrieval/`, `score/` | none (churn only) | W5 | **CLEAR** (classify tools vs engine in W5) |

**Summary:** 3 drift rows total, **all CLOSED** — **D3** via #59, **D2** via tinycortex#63,
**D1** via tinycortex#64; gitlink now `a8e10f7`. No engine drift remains open; **W4 and W7 are
unblocked**. Nothing else drifted. `memory_search` is a mixed module not in the plan's move table —
its `tools/` stay host (agent tools), its `vector`/`scoring` are engine (W5) — flagged for the gap audit.

## D4 — memory_sync corpus (2026-07-09 reclassification, W-SYNC)

The plan's §8 amendment moves the sync engine into the crate, so the blanket **HOST-OWNED
"live sync"** classification above is superseded for engine-mapping sync code. Unlike D1–D3 there
is **no existing crate port to diff against** — the entire `memory_sync/` engine (plus the
`memory_sources` dispatcher/reconcile parts) is port scope. The port baseline for W-SYNC is taken
**fresh from host `main` at W-SYNC.1 branch time**, so ordinary drift cannot accumulate; this entry
exists to (a) record the reclassification and (b) pin the post-port-line commits that already
touched the corpus, so the W-SYNC.1 port provably includes them:

| # | Host commit | Files in corpus | Note |
| --- | --- | --- | --- |
| D4.1 | `c43f79641` (07-03) | `composio/providers/{sync_state,traits}.rs` | TinyAgents-cutover import churn |
| D4.2 | `27b00b539` (07-05) | `sources/rebuild.rs` | test-parity cleanup (1 line) |
| D4.3 | `653e6e143` (07-06) | `memory_sources/sync.rs` (+312) | self-heal drifted content-sha tokens + prune vanished folder items |
| D4.4 | `e456b7799` (07-07) | `memory_sources/{rpc,sync}.rs`, `canonicalize/email.rs`, `composio/providers/{gmail/post_process,notion/source,orchestrator}.rs`, `sources/github.rs` | orchestration-fixes wave |

**Gate:** W-SYNC.1 (crate scaffolding PR) requires this enumeration current as of its branch point
(re-run the scan: `git log --since=2026-06-25 --oneline -- src/openhuman/memory_sync
src/openhuman/memory_sources`); the W-SYNC.3 host flip requires D4 **CLOSED** — every listed commit
(and any accrued since) verifiably contained in the crate port or explicitly waived. Host-retained
parts (schedulers, bus subscribers, RPC wrappers, keychain/OAuth) remain HOST-OWNED as before.

## Closing the ledger (procedure)

For each open row:
1. Branch in `vendor/tinycortex`, port the change (impl + test), PR against `tinyhumansai/tinycortex`.
2. Merge upstream; bump the submodule in a standalone host commit
   `chore(vendor): bump tinycortex — <what> (tinycortex#<n>)`, keeping the `[dependencies]` pin in lockstep.
3. Flip the row to **CLOSED** here; only then may the gated workstream cut over.

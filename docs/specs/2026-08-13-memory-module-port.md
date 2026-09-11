# Porting the memory subsystem into the TinyMemory module

**Goal.** Reach memory only through the loaded `tinymemory` TinyBus module, and
drop `tinymemory`, `tinymemory-api`, `tinymemory-core`, `tinymemory-tinycortex`
and the direct `tinycortex` memory surface from this crate's dependency graph.

**Status.** Audit complete; port staged below.

---

## 1. What is already done

The module architecture is finished and correct. This port is not building it —
it is finishing a cutover that stopped half way.

- `tinymemory-module` ships as a released `cdylib`, pinned with per-platform
  digests in `src/openhuman/modules/registry.rs` (`TINYMEMORY`, v1.0.1).
- `src/openhuman/modules/memory.rs` implements `MemoryProvider` by forwarding
  ~53 methods — the full thirteen-family contract — one for one over the bus,
  lazily, with `memory::api::wire` mapping errors on **both** ends.
- The hard problem is solved. An out-of-crate engine still needs to embed,
  summarise and extract, so three reverse bus services carry those calls back:
  `ChatHost`, `EmbeddingHost` and `RuntimeHost`, served by
  `src/openhuman/modules/memory_host.rs`. Credentials never cross —
  `BusEmbeddingHost::resolve_api_key` returns `None` by construction.
- `src/openhuman/memory/binding.rs` already refuses embedded drivers outright
  and aliases the legacy `tinycortex` driver id onto the module.
- `src/openhuman/memory/api/` is a host-local copy of the contract, and the
  binding and the module client already compile against it rather than against
  `tinymemory-api`.

> ## ⚠ Scope correction (2026-08-15): §2's numbers understate the surface by ~3×
>
> The counts below were derived by grepping for explicit `tinymemory_core::`
> imports. That misses most of the direct-engine access, because
> `memory/mod.rs` **re-exports the engine's modules under host-local paths**:
>
> ```rust
> pub use tinymemory_core::{ chat, global, queue, search, source_scope, store,
>                            tinycortex, tree_policy, tree_source, util, … };
> ```
>
> So a call site written `crate::openhuman::memory::store::chunks::store::list_chunks(…)`
> is engine access that looks exactly like host-local code, and never appears in
> a `tinymemory_core` grep.
>
> Measured properly: **100 files** reach the engine this way — 82 production,
> and **52 of those outside `memory/`**. The heaviest users are
> `store::chunks` (50), `store::create_memory` (31), `store::profile` (26),
> `tree::tree_runtime` (22) and `tree::health` (20), concentrated in the agent
> harness (`archivist`, `learning`, `session`) and `memory/read_rpc/`.
>
> `memory/read_rpc/` is the sharpest example: four files serving a live RPC
> surface straight off the memory database, one of them (`admin.rs`) opening a
> raw `rusqlite::Connection` on the DB path. None of them names
> `tinymemory_core`.
>
> **What this changes.** Stages 2–3 are roughly three times the work the plan
> assumed, and much of it is not "swap a call for a provider method" — whole
> subsystems (`create_memory`, `profile`, `tree_runtime`, `health`) have no
> contract representation and would each need a design decision like the ones
> in §1d. The staging and sequencing still hold; the size estimate does not.
>
> **Measured empirically:** deleting just `store` from that re-export list —
> one of ~24 names — breaks **89 call sites across 51 files** in production
> code alone (`cargo check`, no tests). That is one re-export.
>
> **The facade was deleted first — see §2g.** Converting call sites from an
> incomplete list could never converge while the facade kept generating new
> ones; removing it turns the compiler into the inventory.

## 2. What actually blocks dropping the crates

**Roughly half the host's memory surface never went through `MemoryProvider`.**
It reaches the engine directly, in-process.

| Crate | References | Concentrated in |
| --- | --- | --- |
| `tinymemory_core` | 115 (30 real `use` sites) | `memory/{tools,query,tree,sync,host_impls}` |
| `tinymemory_api::host` | 46 | `config/schema/*`, `inference/`, `cron/`, `integrations/` |
| `tinycortex::memory` | 98 across 40 files | 56 of them **outside** `memory/` |

### 2.1 The consequence is a split brain, not a style problem

`memory_vector_search` calls `list_chunks(&config, &query)`
(`memory/tools/search/vector_search.rs:160`). That resolves the workspace path
and opens the same SQLite database the loaded module has already opened. With
the module driver bound — which is now the only supported binding — the process
runs **two independent engine instances over one database file**. The module is
not authoritative today.

### 2.2 The wire contract has real gaps

These direct call sites are not all "provider calls written the lazy way". Four
things they need have no representation in the thirteen families:

| Missing | Needed by |
| --- | --- |
| **People** — `PeopleStore`, `PersonId`, `Handle`, `Interaction`. No capability family exists. | `memory/tools/people.rs`, `memory/people/` |
| **Chunk-level store access** — `list_chunks`, `get_chunk`, `get_chunk_embeddings_for_signature_batch`, `ListChunksQuery`, `SourceKind` | `tools/search/{vector,hybrid,chunk_context}`, `tools/raw_store/*`, `query/*` |
| **Retrieval primitives** — `fast_retrieve`/`FastRetrieveOptions`, `cover_window`, `search_entities`/`EntityKind`, `RetrievalHit`/`QueryResponse` | `query/{fast_walk,cover_window,search_entities,backend}` |
| **Unified store types** — `MemoryKind`, `MemoryItemKind`, `UnifiedMemory` | `tools/search/hybrid_search.rs`, `tools/raw_store/kinds.rs` |

Each needs a decision: widen the contract, or keep it host-side over data the
provider already returns. Widening is not free — every method added to the wire
is engine semantics both ends must agree on forever.

### 2.3 Some of `tinymemory-core` belongs back in the host

`tinymemory_core::{sync, composio_host, chat, learning_candidate, nlp_host}`
and `memory/host_impls.rs` are orchestration, credentials and scheduling. By
TinyMemory's own README split those are host concerns. They move **back** into
OpenHuman rather than into the module, and `host_impls.rs` is deleted in favour
of the bus services in `modules/memory_host.rs`.

---

## 3. The landmine: two live copies of the embedding signature

`src/openhuman/memory/api/host/` is a near-duplicate of `tinymemory_api::host` —
11 of 17 files byte-identical, 6 diverged. One divergence is dangerous.

`format_embedding_signature` exists in **three** places with **two** behaviours:

| Copy | Form |
| --- | --- |
| `tinymemory_api::host::embeddings` (crate) | `provider={name};model={model};dims={dims}` |
| `tinycortex::memory::store::vectors` | byte-identical to the above, pinned by a parity test in `tinymemory/core/src/tinycortex/parity.rs` |
| `memory/api/host/embeddings.rs` (host-local) | **length-prefixed**: `provider={len}:{name};model={len}:{model};dims={dims}` |

The host-local copy is a *correctness fix* — it stops two distinct
(provider, model) pairs colliding onto one signature, and carries a regression
test for exactly that. It is also, right now, **dormant**:
`src/openhuman/inference/embeddings/provider_trait.rs:20` re-exports the **crate**
version, so every vector written today uses the naive form and matches the
engine.

**This port will make the host-local copy live.** Re-pointing
`inference/embeddings` at `memory::api::host` — which stage 1 does — silently
switches the signature format. Every stored embedding is keyed by that string,
so the effect is not a compile error or a test failure: recall quietly matches
nothing and the system re-embeds the entire corpus.

**Therefore:** the signature change must be landed as its own deliberate change,
upstream in TinyMemory first, so the crate, TinyCortex and the host move
together with a migration for stored vectors — *not* as a side effect of a
re-point. Until then the host-local copy must be reverted to the naive form so
the two copies agree.

Two lesser divergences, both harmless and both resolved in favour of host-local:
`subsystems.rs` defaults the driver to `"tinymemory"` (crate still says
`"tinycortex"`), and `mod.rs` gates test support on `#[cfg(test)]` rather than a
feature.

---

## 4. Staged plan

Each stage compiles and ships on its own.

**Stage 0 — neutralise the landmine.**
Revert `memory/api/host/embeddings.rs` to the naive signature form, keeping the
collision test as `#[ignore]` with a pointer to this section. Open a TinyMemory
issue for the real fix. *No behaviour change; makes every later stage safe.*

> **Ordering constraint — `tinymemory-api` goes last, not first.**
> `tinymemory_core::Config` is `dyn tinymemory_api::host::MemoryHostConfig`
> (`tinymemory/core/src/lib.rs:32`), and `memory/host_impls.rs` implements eight
> of these traits *for host types*. So for as long as `tinymemory-core` is a
> dependency, the host's config must implement the **crate's** trait, and
> re-pointing those references at the host-local copy would not compile. The
> contract crate can only be dropped after the engine crate. Stages 1 and 5 were
> the wrong way round in the first draft of this plan.

**Stage 1 — close the wire gaps.**

*Decision: all four surfaces are pushed down into TinyCortex and exposed through
the TinyMemory contract. None is rebuilt host-side.* The host calls them over
the bus like every other provider method, and the engine stays the single owner
of storage and scoring.

The audit shows this is far less new code than it looks, and **three of the four
surfaces need no migration at all** — a first reading of the file lists suggested
`tinymemory-core` carried a parallel implementation of retrieval and chunks. It
does not:

| Surface | Where it is today | Work |
| --- | --- | --- |
| Retrieval primitives | Algorithms already in `tinycortex::memory::retrieval`. `tinymemory-core/tree/retrieval/{cover,fast,search,drill_down,fetch,source}.rs` are 26–64 line **shims** that add source-scope filtering, limit truncation and logging — a policy layer, not a fork. | Expose only |
| Chunk-level access | `tinymemory-core/store/chunks/store.rs` is a pure delegating wrapper — `engine_config(config)` then straight through to `tinycortex::memory::chunks`. | Expose only |
| Unified store types | Same shim relationship over `tinycortex::memory::store`. | Expose only |
| **People** | `tinymemory-core/people` — 2,138 LOC, its own SQLite database, its own migrations, a workspace-keyed process-global store, and **zero** TinyCortex references. | Genuine migration down into TinyCortex, then a new People capability family |

### 1a. People migration — landed

`tinymemory-core/people` (2,138 LOC, 34 tests) now lives at
`tinycortex::memory::people`, behind a default-off `people` feature that implies
`tokio` (the store shares its connection as an `Arc<tokio::sync::Mutex<_>>`).
`tinymemory-core/people/mod.rs` is a re-export shim, matching `store/chunks`.
Six `tracing::` calls became `log::` — all plain format strings, no structured
fields — so `people` pulls in no dependency TinyCortex did not already have.

**A live bug fell out of it.** The `contacts` gate was never forwarded. The
reader has always lived below this crate, but `contacts` enabled four `objc2`
crates *in the host* — which no file in `src/` names — and never reached
`tinymemory-core`, where the `#[cfg]` is. So the macOS arm of `address_book.rs`
was always compiled out: `SystemContactsSource` returned the empty stub,
`people.refresh_address_book` reported success having seeded nothing, and macOS
paid to compile four unused crates for it.

This is the `voice` (#4901) / `tokenjuice-treesitter` (#4918) failure shape
exactly — an unforwarded gate does not break the build, it silently does
nothing. Fixed by forwarding (`contacts = ["tinymemory-core/contacts"]` →
`tinycortex/contacts`), deleting the host's four unused `objc2` declarations,
and adding `contacts_feature_reaches_the_engine_reader`, which asserts the
property that was missing: that enabling the feature *here* changes what the
reader does *there*. A `cfg!(feature = ...)` self-assertion would have passed
throughout the bug.

**Verification.** TinyCortex: 34 people tests pass; default and `contacts`
builds clean. Host: builds clean both ways; feature-forwarding gate passes;
`openhuman::memory::` is 711 passed / 26 failed / 1 ignored against `main`'s
710 / 26 / 0 — the 26 are pre-existing and identical on a clean `main`
checkout (they need the module artifact, which is not fetched locally), so this
adds one passing test and one deliberately ignored one, and no new failures.

### 1b. The People capability family — landed in the contract

`Capability::People` is family fourteen, appended (never inserted — declaration
order is bit order in the `Capabilities` bitset, so moving a variant would
silently re-interpret an already-transmitted set). `CONTRACT_VERSION` goes
`(2, 0)` → `(2, 1)`.

**A new family, not methods on an existing one — the version rule forces this.**
Adding these calls to `MemoryEntities` would have been a **major** bump, because
a new method on a family a driver may already advertise is breaking: negotiation
cannot protect a caller from a method an older driver never implemented. A new
family is a minor bump, and an older driver simply does not advertise it.

`MemoryPeople` carries seven methods, derived from the seven agent tools and
four RPC controllers that exist today rather than guessed at: `list_people`,
`get_person`, `resolve_handle`, `add_handle_alias`, `score_person`,
`record_interaction`, `seed_from_address_book`. Its types are the contract's
own — TinyCortex's `Person`/`Handle`/`Interaction` never cross — and identity
travels as an opaque `PersonRef` string, since the contract does not promise
every engine identifies people by UUID.

Wired through: both contract copies, `NullMemoryProvider`, the `MemoryGuard`
decorator (`GuardedPeople`), and the recording test fixture. In the guard,
`resolve_handle` takes the **write** tier check when `create_if_missing` is set
and the read check otherwise — classifying the whole method as a read would have
handed a `readonly` operator a working insert through the back door.

**A drift guard now holds the two contract copies together.** There are two
copies of this contract — the host-local one the module *client* compiles
against, and `tinymemory-api` which the module *service* compiles against — and
they meet only over a bus, where a mismatch is not a type error but a method
never called or a capability filtered away on one side. That duplication already
produced one live defect (§3). Two tests now pin the families and the version
across both copies; both were verified to actually fail by temporarily diverging
a wire string, then to go green again. They are deleted with the
`tinymemory-api` dependency in stage 5, when one copy remains.

**Verification.** `openhuman::memory::api` 190 passed / 0 failed;
`openhuman::memory::guard` 56 / 0; `core::all` 91 / 0; `openhuman::memory::`
back to exactly the 26 pre-existing failures with no new ones; `cargo fmt`
clean. (The full `--lib` run aborts on a pre-existing stack overflow in
`agent::harness::session::runtime`, identical on a clean `main` checkout.)

### 1c. Engine implementation, module service, host client — landed

**Not in `tinymemory-tinycortex`.** That adapter holds only an
`Arc<dyn Memory>` and documents its own scope as the mandatory three, because
the optional families need a host's configuration. The implementation belongs
to `tinymemory-module`'s `ModuleMemoryProvider`, which already holds
`workspace_dir` and implements the other ten.

**Conversions destructure; they do not round-trip through serde.** The module's
`Self::cross` helper is a serde value round-trip, and it would have compiled
and then failed at runtime on the first call: the engine's `Interaction` names
its timestamp `ts` where the contract names it `at`. Explicit destructuring
makes a renamed or added field a compile error instead — the rule
`tinymemory-tinycortex::convert` already follows.

Two smaller decisions worth keeping:

- A malformed `PersonRef` is `Invalid`, not `NotFound`. `NotFound` would tell a
  caller their id was well-formed but absent, sending them to look for a deleted
  person rather than at the id they built.
- Ranking sorts with `total_cmp`, not `partial_cmp`. A NaN from a degenerate
  score makes `partial_cmp` return `None`, and `sort_by` on a non-total ordering
  may panic or produce garbage order.

Service side: seven methods on `ai.tinyhumans.tinymemory.Memory`, with
`ListPeople` size-checked like the other list-returning methods — `limit` bounds
the count but not the bytes. Host side: seven forwards through `module_call!`
and an `as_people()` accessor.

**The nested TinyCortex submodule was fast-forwarded** (`be7b395` → `566804c`,
verified as an ancestor first) so the module crate can actually build and test
against the engine change. That pointer bump is part of the release anyway.

### Release-ordering hazard — read before shipping stage 2

`ModuleMemoryProvider::capabilities()` answers `Capabilities::all()`
**statically**. That set grew with the contract; the *artifact* only grows when a
release is cut and `modules/registry.rs` is re-pinned. Between those moments the
host over-claims, and `verify()` logs the disagreement without narrowing the
advertised set.

`people` is in that window now: served by the module source in this tree, not by
the pinned `1.0.1` artifact. It is **inert today** because nothing in the host
reaches `as_people()` yet. It stops being inert the moment the people RPC
handlers are routed through the driver, so **that change and the module release
must land together**. Documented at the `capabilities()` call site too.

### 1d. The `Chunks` and `Retrieval` families — landed

Families fifteen and sixteen, contract now `(2, 1)` with three additions (one
minor bump covers all three — capability negotiation is what makes each safe).

- **`MemoryChunks`** — `list_chunks`, `get_chunk`, `chunk_embeddings`. A
  deliberately lower-level surface than the rest of the contract: it exists so a
  host doing its *own* ranking (cosine + MMR, hybrid keyword/vector) can get the
  rows without reaching around the driver into the engine's tables — which is
  the split-brain this port exists to end.
- **`MemoryRetrieval`** — `fast_retrieve`, `cover_window`, `search_entities`.

Three decisions worth keeping:

**Source scope had to become an explicit wire argument.** `tinymemory-core`'s
in-process entry points read it from a **task-local**. That task-local belongs to
the host's task and does not exist on the module's side of a bus call, so it
would have read as `None` there — and `None` means *unrestricted*. A
per-profile source gate would have failed open, silently, on every scoped
retrieval. So `cover_window_scoped` and `fast_retrieve_scoped` were added
alongside the ambient-scope originals (mirroring TinyCortex's own
`cover_window_scoped`), and every scoped method on the wire takes `scope` as an
argument and never infers it.

**Entity kinds travel as strings, not as an enum.** The engine's `EntityKind` is
`#[non_exhaustive]` and has grown twice. A closed enum on the wire means the
first time the engine emits a kind this build has not heard of, the **response
fails to deserialize** — a new entity category would break retrieval outright
rather than showing up as an unfamiliar label. Responses therefore carry an open
snake_case vocabulary. Requests are the opposite case and *are* validated: an
unknown kind in a filter is `Invalid`, because silently matching nothing is
indistinguishable from a genuine empty result.

**`chunk_embeddings` sorts its result.** The engine returns a `HashMap`, whose
iteration order varies per process; an otherwise-identical call would return a
differently-ordered list. It is also the largest thing this interface returns —
a 1536-dimension vector is roughly 10 KiB of JSON — so it is size-checked and
refused by name rather than truncated, since a short batch is indistinguishable
from "those chunks have no vector".

**Verification.** Module crate 34/0 · host `memory::api` 190/0 ·
`memory::guard` 56/0 · `core::all` 91/0 · `openhuman::memory::` failing set
byte-identical to the pre-existing 26 · `cargo fmt` clean across all four
crates.

### Still open in stage 1

- Routing the host's people / chunk / retrieval RPC + agent tools through the
  driver (stage 2) — that is what makes these three families load-bearing and
  what ends the split brain.
- A module release, the digest update in `modules/registry.rs`, and the
  TinyMemory-side submodule pointer commit.

**Why People moves rather than staying put.** `tinymemory-core` survives this
port — it is the module's own implementation crate, it just stops being an
*OpenHuman* dependency — so leaving People there would compile. But the contract
defines a capability and each engine implements it; a second engine binding in
TinyCortex's place must bring its own People store. Storage belongs to the
engine, which is exactly the split that makes the contract engine-neutral.

Then: widen `MemoryProvider` and the capability set, extend the module service
and the host client in `modules/memory.rs`, cut a TinyMemory module release and
update the digests in `modules/registry.rs`.

This is the largest stage and the only cross-repo-blocking one — it needs a
published module release, taken verbatim from the release's `checksum.toml`,
never recomputed from a local build.

**Stage 2 — cut the direct engine calls over.** *(in progress)*
Rewrite the 30 `tinymemory_core` call sites in `memory/{tools,query,tree}` onto
the provider. Ends the split brain.

### 2a. A guard bug caught before any call site moved

The `GuardedChunks` / `GuardedRetrieval` decorators written in stage 1 forwarded
the caller's `scope` argument **unchanged**. That is the exact widening leak
`GuardPolicy::narrow_scope` exists to close, and its own docs record the earlier
version of it: with a pass-through, a source-restricted turn can name a
collection its restriction excluded and have that become the sole query
predicate, so the restriction vanishes.

Fixed to intersect via `narrow_scope`, matching `GuardedTree::query_source`.
Four regression tests added (`families_tests.rs`), and each was verified to
**fail** against the pass-through version before being kept — a scope test that
cannot fail is worse than none.

### 2b. Call sites converted

| Tool | Was | Now |
| --- | --- | --- |
| `memory_vector_search` | `list_chunks(&config, …)` + `get_chunk_embeddings_for_signature_batch` | `as_chunks()` |
| `memory_chunk_context` | `get_chunk` + `list_chunks` | `as_chunks()` |
| `memory_store_raw_chunks` | `list_chunks` | `as_chunks()` |
| `memory_tree` walk | `fast_retrieve` | `as_retrieval()` |
| `memory_tree_cover_window` | `cover_window` | `as_retrieval()` |

`memory_vector_search` is the case §2.1 names: it resolved the workspace path
and opened the same SQLite database the loaded module already had open. It no
longer touches the engine.

Two of these dropped their `load_config_with_timeout()` entirely — the config
load existed only to reach the database.

**A test moved to `#[ignore]`, deliberately.**
`raw_chunks::execute_success_path_returns_json_array` was a pure-SQLite test: it
opened the workspace store in-process and read an empty table. That *is* the
split brain. With no module artifact the binding now falls back to the null
driver and the tool refuses — the correct answer, not a regression — so the test
joins the module-backed set (`OPENHUMAN_MODULE_PATH`, own process), the same
pattern the `tinydocs` tool tests use.

**Verification.** `openhuman::memory::` 716 passed, failing set byte-identical
to the pre-existing 26; `memory::guard` 60/0 (four new).

### 2c. Second batch converted

`memory_tree_search_entities`, `memory_store_raw_search` (both onto
`as_retrieval()`), and `memory_tools_list` (a stale doc link only).
`memory_chunk_context`'s remaining `chunk_source_allowed` call was re-pointed at
the host's own `memory::source_scope` path, which is how `guard/policy.rs`
already reaches it — the predicate is host *policy* over a host task-local, so
relocating it properly is stage 4.

**Entity-kind validation moved into the driver**, as flagged. The host used to
`EntityKind::parse` before touching disk; the vocabulary is open on the wire
(§1d) and the driver is its authority, so a host-side copy would either reject a
kind the engine understands or drift out of date. The cost is named rather than
hidden: a bad `kinds` value used to fail with no workspace and now needs a bound
driver, so `execute_rejects_invalid_kind_after_validation` became a
module-backed test rather than being quietly relaxed.

**Eight tools are now off the engine entirely** — `vector_search`,
`chunk_context`, `raw_chunks`, `raw_search`, `tool_memory/list`, `fast_walk`,
`cover_window`, `search_entities` name no engine crate at all.

**Six tests moved to `#[ignore]`, all for the same reason and none of them
cosmetic.** Each asserted a success path by reading the workspace store
in-process — which *is* the split brain. Two also ran the tool and then called
the engine directly on the same workspace to assert both agreed; that second
reader is precisely what this port removes, so the parity half is gone rather
than reworked. This is a real loss of local coverage until the module release
lands, not a cleanup.

### 2d. The retrieval trio, `interaction_count`, and a merge from `origin/main`

**Three methods added to `MemoryRetrieval`**, unblocking `backend.rs` and its
three tool wrappers: `retrieve_source`, `retrieve_children`, `retrieve_leaves`.

**The names are deliberate, and one was forced.** `MemoryTree` already has
`drill_down` and `query_source` with *different* semantics — the tree family
returns a node and its direct children, or the raw chunks filed under a source;
these return ranked hits across the summary tree, several levels deep. Beyond
the ambiguity, both families are served on **one bus object**, so `drill_down`
collided outright and would not compile. Renaming the trio consistently
(`retrieve_*`) resolves both.

`query_source_scoped` joins `fast_retrieve_scoped` and `cover_window_scoped` in
`tinymemory-core` for the same reason as §2c: a task-local scope does not cross
a transport, and reading it as absent means unrestricted.

**`RankedPerson::interaction_count`** added, restoring the field the people RPC
payload carries. It is worth carrying for its own sake: a score alone cannot be
read honestly, since 0.9 from three exchanges and 0.9 from three hundred are the
same number and very different facts.

**Merged `origin/main`** (90 commits). One thing to know: the merge commit
correctly recorded `tinyagents → 30d6b3b` and `tinyflows → c242184`, and then
the **auto-commit hook committed the stale worktree submodule pointers straight
back**, reverting both gitlinks and breaking the build with an unresolved
`tinyagents::harness::artifacts`. Restored from the merge commit and the
submodules checked out to match. Worth watching for on any future merge in this
repo — the hook cannot tell a stale submodule worktree from an intended change.

**Now converted:** `backend.rs` (a doc comment is all that mentions the engine),
plus `query_source`, `drill_down` and `fetch_leaves`. Five more success-path
tests became module-backed, for the same reason as the earlier six — each read
the workspace store in-process, and three carried a direct-engine parity half
that has no second reader to agree with any more.

**Verification.** `memory::` 708 passed / 26 failed — failing set identical to
the post-merge baseline, no new failures · `memory::api` 190/0 ·
`memory::guard` 60/0 · `core::all` 91/0 · module crate 34/0 · `cargo fmt` clean
· both contract copies byte-identical apart from the intentional doctest path.

### 2e. People converted

`people/rpc.rs`, `people/schemas.rs`, `tools/people.rs` and a **third caller the
first survey missed** — `flows/tinyflows/memory_adapter.rs`, which called
`rpc::handle_list` directly — all now reach the driver's people family. None of
the four names an engine crate.

**`interaction_count` moved from `RankedPerson` to `PersonScore`.** `handle_score`
needs it too, and duplicating the field would have let the two copies disagree.
It belongs on the score anyway: the score and the sample size it was computed
from should travel together, so every caller that gets one gets the other.

Two deliberate behaviour changes, both surfaced rather than absorbed:

- **`person_id` is no longer validated as a UUID host-side.** `PersonRef` is
  opaque by contract — the driver issues the id and owns its format — so a
  host-side UUID check would reject a driver that identifies people some other
  way. `parse_person_id_rejects_non_uuid` was inverted into
  `parse_person_id_accepts_any_non_empty_token`, with a companion asserting that
  a *missing* id is still the host's to reject: that is a malformed call, not an
  unrecognised identity.
- **`permission_denied` in `people.refresh_address_book` is now always `false`.**
  The contract reports a host without an address book, or without permission,
  as `seeded: 0` rather than a distinct error — both mean the same thing to a
  caller, and the alternative leaks a platform detail into an engine-neutral
  contract. The field is kept so the published shape does not change, but
  surfacing "grant Contacts access" now needs a host-side permission probe.

**The RPC tests were rewritten, not gated.** They used to build a real
in-memory `PeopleStore`; they now drive a small fake `MemoryPeople`. That is
better coverage, not worse: ranking and scoring moved into the engine and are
tested there, so what is left host-side is the published JSON shape and the fact
that the driver's order is passed through rather than re-sorted — and there is
now an explicit test that the host does **not** re-sort, since a host-side sort
would silently override the ranking authority.

### 2f. People's split brain closed at the store, not just the call sites

Converting the callers left the *store* still being opened host-side. Four sites
seeded a process-global that nothing read any more, so the host held a second
connection to `<workspace>/people/people.db` — the file the module owns — purely
to populate a global no handler consulted. All four are gone:

| Site | Was |
| --- | --- |
| `core/runtime/context.rs` | boot seed under `StoreInitPlan.people` |
| `security/credentials/ops.rs` | rebind after login, and after logout |
| `desktop/app_state/ops.rs` | rebind on active-user switch |

`CoreContext::people()` and the `StoreInitPlan.people` field went with them. The
active-user rebinds needed no replacement: people resolves through the memory
binding now, and `rebind_default_workspace` already moves that.

**No host site opens the people database any more.** The engine still compiles
it in — that is where it belongs.

**Three context tests were removed rather than repointed**, and it is worth
being precise about what that costs. `people_store_is_isolated_per_context_workspace`
and `rebind_workspace_updates_context_store_resolution` proved per-context
workspace isolation *using people as the example*; that property is proved
unchanged by `memory_binding_is_isolated_per_context_workspace` and
`rebind_workspace_updates_context_memory_binding`, which is what people resolves
through now — so this is redundancy removed, not coverage lost.
`people_rpc_uses_scoped_context_store` is different: it asserted a scoped
`people_resolve` wrote workspace A and not B by opening **both stores directly**.
There is no second reader to check against any more, and the isolation it tested
belongs to the binding. `degraded_context_rejects_workspace_bound_stores` now
asserts `workspace_dir()` directly — the gate every workspace-bound store passes
through, and what `people()` was standing in for.

**Verification.** `memory::` 713 passed / 26 failed, no new failures ·
`core::runtime` 27/0 · `core::all` 91/0 · `memory::people` 13/0 ·
`security::credentials` 183/0 · `desktop::app_state` 32/0 · `cargo fmt` clean.

### 2g. The re-export facade is gone

`memory/mod.rs` no longer re-exports **any** engine module. All ~24 names
(`store`, `queue`, `global`, `chat`, `search`, `tinycortex`, `source_scope`,
`util`, …) were removed and every call site now says `tinymemory_core::`
explicitly — ~190 references across 86 files in `src/`, plus 14 integration
tests and 4 binaries.

This is not a conversion: **no behaviour changed**, because each rewritten path
resolved to exactly the symbol it now names. What changed is visibility. Before,
`crate::openhuman::memory::store::chunks::store::list_chunks(…)` was engine
access indistinguishable from host-local code; a `tinymemory_core` grep returned
30 files and the truth was 100. Now `grep tinymemory_core src/` **is** the
inventory: **127 production files**, plus 94 naming `tinycortex`.

Flat *type* re-exports (`memory::MemoryCategory`, `memory::Memory`) were kept
and re-pointed. They still have to move to `memory::api`'s equivalents, but a
type name hides nothing the way a module tree does.

**A latent test bug surfaced and was fixed.** `agent::learning::startup`'s tests
build a real `MemoryClient`, which needs the host seams wired — and that module
never called `install_for_tests`. It passed only when another test in the same
binary happened to run first; alone, or filtered to that module, it failed with
"no EmbeddingHost installed". Verified pre-existing (`git log -S` shows the call
was never there, and no commit in this work touched `host_impls.rs`, the only
caller of `set_embedding_host`). The whole-suite runs never caught it because
the pre-existing stack overflow in `agent::harness::session::runtime` aborts
that binary first. One `Once`-guarded call fixes it: 144 → 145 passing.

### 2h. The measured remaining surface, after the facade came down

`grep tinymemory_core src/` is now the inventory. By engine module:

| Module | Refs | What it is, and what it needs |
| --- | --- | --- |
| `store::chunks` | 52 | Partly `MemoryChunks` already; `memory/read_rpc/` uses `with_connection` and raw SQL and needs its own design |
| `store::create_memory` | 31 | **30 of these are tests.** Only *one* production site constructs a `MemoryClient` — the "31 per-site decisions" reading was wrong. Test constructions are not a split brain and go when the dep does |
| `store::profile` | 26 | The learning/profile subsystem (`ProfileFacet`, `FacetState`, `UserState` + SQL). No contract representation; needs a family design like §1d |
| `global` | 40 | The process-global memory client — the same shape as the people global just deleted |
| `queue` | 37 | The ingest job queue |
| `tinycortex` | 26 | Direct engine reach-through |
| `store::safety` | 14 | **Blocked, see below** |
| `store::{UnifiedMemory,trees,segments,fts,content}` | ~45 | Engine internals with no contract analogue |

**`store::safety` cannot simply come home.** TinyMemory's README puts redaction
on the host, and the 2,065-LOC PII/secret detector currently sits in the engine
— but the engine *uses* it on its own write paths in 14 places (`store::kv`,
`goals::store`, `persona`). Moving it host-side would fork it, and a forked
redactor is the same class of hazard as §3's forked embedding signature, with
worse consequences. It is a third-crate extraction (the `tinydocs` /
`tinywallet` shape), not a move.

**Production vs test, measured.** The raw counts mix both. Split properly:
**342 production references across 131 files**, and 125 test references. The
split matters per cluster — `create_memory` is 1 production / 31 test, while
`store::safety` is 14 / 0 and `store::profile` is 24 / 2. Test-side engine use
is not a correctness problem; it disappears with the dependency.

**Honest sizing for the rest.** Stages 2–5 need, at minimum: a contract family
for the profile/learning subsystem; a decision for each `create_memory` call
site; a design for `memory/read_rpc/`'s raw-SQL surface; the `global` and
`queue` seams; a `tinysafety` extraction; then the module release, the
`tinymemory-api` retirement (§ordering constraint) and the dep drop. That is a
programme measured in weeks, not a tail-end sweep — and the number is now
trustworthy, which it was not before §2g.

### 2i. `hybrid_search` — the worst split brain, removed

`memory_hybrid_search` called `UnifiedMemory::new(&config.workspace_dir, …)`:
it constructed **an entire second engine** over the workspace the loaded module
already owns. Not a stray query — a whole store instance, with its own
embedder and its own SQLite handles.

It needed scored hits with their signal breakdown so it could re-rank under a
weight profile, which `MemoryRecall` does not expose — it returns ranked
entries and keeps its scoring private. Added
`MemoryRetrieval::recall_namespace_scored`, returning
`NamespaceMemoryHit` (whose `score_breakdown` the contract *already* defined),
so re-ranking is host policy over engine signals rather than a second retrieval
implementation. Also added `MemoryChunks::chunk_detail`, a one-call inspection
view — four accessors would have been four bus round trips per rendered row.

**Adding methods to `MemoryRetrieval` keeps the version at (2, 1)**, which looks
like it violates the major-bump rule. It does not: the rule protects *deployed*
drivers, and `(2, 1)` has never shipped — `Retrieval` itself is new in it. Once
the module release goes out, this stops being true.

`MemoryClient::unified_handle` was added beside the existing `memory_handle`
for the module's scored-recall path, documented as the narrower-surface
exception it is.

### 2j. Two pre-existing test defects, diagnosed and fixed

Both surfaced because converting call sites changed which tests run together.

- **`sync_pipeline_e2e_tests` was flaky**, alternating pass/fail across
  identical runs (708/707). It counted events published across two tinybus task
  hops after a single `yield_now()`. The file already had a `wait_for` helper
  written for exactly this, with a doc comment explaining the two-hop problem —
  three call sites just did not use it. One of them additionally needed to wait
  for the **terminal** `completed` stage rather than any stage event.
- **The same module never installed the host seams**, so it passed only when
  another test in the binary had. Same defect as `agent::learning::startup`
  (§2g), same one-line fix.

Verified stable: 708 passed across three consecutive full runs, where it
previously alternated.

### 2k. The `Profile` capability family

Family seventeen. `store::profile` + the `global::client_if_ready()` calls that
existed only to reach `profile_store()` were one cluster, not two — the learning
and archivist subsystems read and write the engine's facet table directly.

`MemoryProfile` carries eleven methods over `ProfileFacet` / `FacetType` /
`FacetState` / `UserState`. Three decisions worth keeping:

- **The host owns the learning; the driver owns the rows.** `ProfileFacet`
  carries a `stability` and a `state` the driver never computes — it records
  what the host's stability detector decided. Extraction, scoring, promotion and
  eviction all stay host-side; this family is only the persistence seam beneath
  them.
- **`user_state` outranks the score, and that is a contract obligation.**
  `Pinned` stays active however low stability falls; `Forgotten` stays dropped
  however much new evidence arrives — a user who says "forget that" must not
  have it re-learned. `drop_facets_below` is documented as required to honour
  both, so a future driver cannot quietly sweep against an override.
- **`workflow_identity_matches` returns `bool`, not `Result<bool>`**, matching
  the engine method it replaces. Every caller is an "is this row the user?"
  predicate whose only sane reading of a failure is *no*; threading a `Result`
  through them invites an `unwrap_or(true)` somewhere. The guard, the wire and
  the client each answer `false` on refusal, absence and transport failure
  respectively — the one place the contract deliberately swallows an error, and
  it says so.

`ProfileStore`'s methods are synchronous and hold a `parking_lot::Mutex` across
SQLite, so every module-side call goes through `spawn_blocking` rather than
being awaited on the runtime thread.

Wired through both contract copies, the null driver, the guard, the fixture, the
module implementation, the bus service and the host client. **Call sites are not
converted yet** — that is the next step, and it is what makes the family
load-bearing.

### 2l. The learning subsystem converted onto `MemoryProfile`

`FacetCache` reads and writes through the driver now, and the subsystem went
async with it: `load_learned_from_cache`, `StabilityDetector::rebuild`,
`ProfileMdRenderer::render`, and every call site in `schemas`, `tools`,
`scheduler` and `startup`.

**Async-ifying removed work.** `render()` was wrapped in `spawn_blocking`
specifically to keep in-process SQLite off the executor. With the store behind
the module there is no blocking I/O left to move, so the hop is gone.

**No coverage was lost.** The ~50 learning tests built real in-memory SQLite
stores. Rather than park them on `OPENHUMAN_MODULE_PATH`, they drive an
in-memory `MemoryProfile` (`agent/learning/test_profile.rs`) — those tests are
about stability scoring and prompt rendering, not persistence. 145 pass.

Two defects the tests caught in this work:

- **The fake's `drop_facets_below` was wrong**, and the existing assertions
  failed on it. The engine sweeps only rows already in `FacetState::Dropped`,
  and protects only `Pinned`. The contract doc had **overstated the guarantee**
  by claiming `Forgotten` was protected too; corrected, with the reason the
  asymmetry is deliberate — a Forgotten facet is already Dropped and is meant to
  be collected.
- **`test_profile` was first written `#[cfg(test)]`**, which integration tests
  cannot see — the exact trap `ProfileStore::for_tests` documents. Now
  `#[doc(hidden)] pub`.

**The bypass allowlist shrank.** Five entries justified by *"the contract has no
profile family"* are gone. One was added — a boot-time `binding::for_workspace`
that resolves a **guard** (not a raw client) for a known workspace, as
`active_memory_guard`'s own fallback does — with that reason recorded in both
the test and `docs/specs/memory-guard-allowlist.md`.

### 2m. The `Arc<dyn Memory>` seam — attempted, reverted, and why

`AgentExperienceStore` looked like the next bounded conversion: it uses only
`get` / `list` / `store`, all in `MemoryCore`. It was converted, and then
reverted.

`agent/harness/session/turn/core.rs` builds experience stores from the
session's own `Arc<dyn Memory>` **and** from a second, *shared* experience
memory. The guard is per-workspace; the session may legitimately hold two
memory handles. Converting only the `ops.rs` door would have left
`AgentExperienceStore` with two constructors, one guarded and one not — which
the bypass allowlist would rightly flag, and which is worse than the current
state.

So `Arc<dyn Memory>` is not a call-site cluster at all: it is a **seam**
threaded through the session builder, the flows adapter and the experience
store, and it has to be replaced as one design rather than file by file. That
is the largest single item left, and it is the reason the remaining `global`
sites cannot simply be deleted the way the people global was.

### 2n. A way through the `Arc<dyn Memory>` seam

The seam looked un-splittable in §2m because every consumer is *handed* a
memory handle by `build_tools(memory: Arc<dyn Memory>, …)`, which is fed from
the session builder. Converting one consumer meant converting the constructor,
which meant converting the builder.

There is a way through, and this port already established it: **a converted tool
resolves the guard itself**. `vector_search`, `chunk_context`, `raw_chunks`,
`fast_walk` and the rest hold no handle — they call `active_memory_guard()` per
invocation. Applying that to a seam consumer removes its dependency on the
constructor parameter entirely, and the parameter dies of disuse once the last
consumer stops reading it.

`memory_recall`, `memory_store` and `memory_forget` are converted on that
pattern: each is now a unit struct (or holds only its `SecurityPolicy`), and
`build_tools` no longer passes them a handle. Holder count 33 → 30.

Two details the conversion surfaced:

- **`recall`'s options changed shape.** The engine trait takes a borrowed
  `RecallOpts`; the contract takes `&OwnedRecallOpts` plus an explicit `scope`.
  `None` is passed for scope, which is not "unrestricted" — the guard
  intersects it with the ambient allowlist, so it can only narrow.
- **`store` gained a taint argument.** The engine's `store` has none; the
  contract requires one because a driver that could default provenance could
  launder external content as internal. The tool passes `MemoryTaint::default()`
  as a *request*, and the guard stamps the effective value.

Their engine-backed tests join the module-backed set (the read-back goes through
a real `UnifiedMemory`, so those assertions need the artifact). `name_and_schema`
stopped needing a store at all.

### 2o. The seam's root is four call sites, not thirty

Triaging the remaining `Arc<dyn Memory>` holders against the contract:

| What they need | Files | Status |
| --- | --- | --- |
| `store` / `get` / `forget` / `list` / `recall` | most | **covered** — `MemoryCore` + `MemoryRecall` |
| `namespace_summaries()` | 7 | **covered** — the contract's `namespaces()` has the identical signature and return type; it is a rename |
| `count()` | 3 | **test-only.** No production call site uses the trait's `count` |
| `recall_relevant_by_vector()` | 0 | unused anywhere |
| `memory_handle()` | 4 | **the root** |
| `tool_memory_store(…)` / `preferences::…` | 3 | engine helpers taking `&Arc<dyn Memory>` |

So the seam is not thirty independent conversions. Nearly every holder just
*receives* a handle; only four production sites **mint** one —
`agent/experience/ops.rs` (×2), `agent/harness/session/builder/factory.rs`,
`flows/bus.rs` and `flows/tinyflows/memory_adapter.rs`, each calling
`MemoryClient::memory_handle()`.

Convert those four to hand out the guard and the downstream holders change type
mechanically, because what they call is already in the contract. That is the
finish line for the seam, and it is a much smaller target than the holder count
suggests.

Two consumers need engine helpers that take `&Arc<dyn Memory>` —
`tool_memory_store` and `preferences::recall_related_preferences`. Those are
host-layer helpers living engine-side; they come home with stage 4 rather than
being wrapped.

### 2p. ⚠ The seam root is blocked on an architectural decision, not on typing

Converting the four `memory_handle()` roots turns out not to be mechanical, and
the reason is worth stating precisely because it is **not in the original plan
and it gates the rest of the port**.

`agent/harness/session/builder/factory.rs` does not take a handle to the
workspace's memory. It **constructs its own engine instance**:

```rust
let session_memory = memory_store::factories::create_session_memory_with_local_ai(
    …, &config.workspace_dir, &memory_subdir,   // "memory" | "memory-<profile-id>"
)?;
let archivist_connection = session_memory.sqlite_connection;
let memory: Arc<dyn Memory> = Arc::from(session_memory.memory);
```

Two things fall out, and the module architecture accommodates neither:

1. **Per-profile memory subtrees.** A profile with `dedicated_memory` gets its
   own store at `<workspace>/memory-<id>`, which is the whole point of that
   feature — isolation. The contract and the binding address a **workspace**
   (`binding::for_workspace(workspace_dir, cfg)`); there is no notion of "open
   the store rooted at subdirectory X". One loaded module serving one store per
   workspace cannot express this.
2. **A raw `sqlite_connection` handed to the archivist.** `ArchivistHook::new`
   takes the live SQLite connection out of the session's memory. A connection
   cannot cross a bus, so there is no forwarding fix — the archivist's storage
   has to be re-homed, not re-routed.

There is also a third store in play: a dedicated-memory session *additionally*
holds `shared_experience_memory`, a handle to the **global** store, so
pre-profile unstamped experiences stay recallable. So one session can legitimately
hold two stores plus a raw connection.

**This is a design decision, not a conversion.** The options are roughly:
extend the contract so a driver can serve named stores within a workspace
(a real widening, and the module would need to load or multiplex per subtree);
or bind one module per memory subtree; or re-scope `dedicated_memory` so
isolation is expressed inside one store rather than by a separate database. Each
changes user-visible behaviour or the module's lifecycle, and none should be
picked without an explicit call.

Everything downstream of these four sites is mechanical once that is settled —
§2o shows the receivers need only what the contract already has. But the roots
themselves cannot be converted until per-profile memory has an answer.

### 2q. An in-memory provider, so conversions stop costing coverage

Every seam conversion had been paying the same toll: the consumer's tests handed
it a real `UnifiedMemory` over a temp dir and asserted a genuine round trip, and
converting to the guard turned them into `#[ignore]`d module-backed tests. That
is ~36 tests parked so far.

`memory/guard/in_memory.rs` ends that. It is a `HashMap` behind a mutex
implementing the **mandatory three**, so it can be wrapped in a *real*
`MemoryGuard` — `guarded_in_memory()` returns both. A converted consumer's tests
keep their round-trip assertions, and gain the policy layer on the path where
production has it.

`RecordingProvider` could not serve: it records calls and answers empty, which
proves a call was made but never that the data came back.

Two deliberate limits, stated in the module so nobody mistakes it for the
engine: `recall` is a substring match, not ranked retrieval (a test about
*ordering* must use the real engine), and `list`/`namespaces` sort explicitly
because a `HashMap`'s iteration order would make otherwise-identical assertions
flaky. It is `#[doc(hidden)] pub`, not `#[cfg(test)]`, for the integration-test
reason this port has already tripped over twice.

**First use: `flows/bus.rs`.** The run-digest subscriber now resolves the guard,
and its `store_with_taint` call became `store` — the contract carries taint on
the one door, so the engine trait's second door is unnecessary. All 33 bus tests
keep their assertions and pass.

The bypass allowlist lost its two `flows/bus.rs` entries with it.

**A third order-dependent test defect surfaced** — `flows::ops` tests build an
agent, which constructs a memory client, which needs the host seams; they had
never installed them and passed only on ordering. Same one-line fix as
`agent::learning::startup` and `sync_pipeline_e2e_tests`. That is three
independent instances of the same latent defect this port has now found and
fixed.

### 2r. Both flows roots converted

`flows/tinyflows/memory_adapter.rs` and the `flows/memory_tools.rs` helpers it
delegates to (`cross_flow_recall`, `FlowMemoryRecallTool`,
`FlowMemoryRememberTool`) now go through the guarded driver. Two of the four
`memory_handle()` roots are gone; the remaining two are the ones downstream of
the per-profile decision in §2p.

Three shape changes, each removing a door rather than adding one:

- **`namespace_summaries()` → `namespaces()`.** Identical signature and return
  type; the contract simply names it differently. This is what makes the seven
  files that call it mechanical.
- **`store_with_taint(…)` → `store(…)`.** The contract carries taint on the one
  store method, so the engine trait's second door has no counterpart and needs
  none.
- **`is_potentially_untrusted` stopped taking a `MemoryEntry`.** Two entry types
  are in play during the port, and the predicate needs neither — it reads a
  namespace and a key. It takes those now, so callers on either side use it
  without conversion, and the signature states what it depends on.

The bypass allowlist lost both `memory_adapter.rs` entries. Across this port it
has now shed nine: five profile/facet, two `flows/bus.rs`, two
`memory_adapter.rs` — against one added (a boot-time guard resolution, with a
reason). Its own rule is that it may shrink and must never grow.

### 2s. The tool registry stopped taking a memory handle

`all_tools` / `all_tools_with_runtime` took an `Arc<dyn Memory>` and threaded it
into exactly **two** tools: `SavePreferenceTool` and `ToolStatsTool`. Each used
one mandatory-family method (`forget`, `list`). Converting those two to resolve
the guarded driver per call made the parameter dead, and dropping it collapsed
**both remaining engine-construction sites** in one step:

| Site | Was |
| --- | --- |
| `channels/runtime/startup.rs` | `create_memory_with_local_ai(...)`, plus a second fallback construction with `embedding_provider = "none"` when the embedder failed (#3712) |
| `runtime/node/ops.rs` | `tinymemory_core::store::create_memory_with_local_ai(...)` |

The #3712 fallback goes away with the construction it protected: there is no
embedder to fail to build here any more, because the host no longer builds a
store at all. The degradation it bought — channels still start when the
embedding provider is misconfigured — now belongs to the driver, which is a
better place for it: it applied to one of the four construction sites, and the
other three had no such protection.

`ChannelRuntimeContext.memory` became `Arc<MemoryGuard>` in the same change,
and `build_memory_context` with it.

### 2t. `preferences` came home, and cost the contract nothing

`tinymemory_core::preferences` was host policy living in the engine: which two
namespaces the lanes use, how many standing preferences a prompt may carry, and
the similarity floors for Lane-B recall and the contradiction check. A second
engine would have had to reimplement all of it identically or the product would
change underneath it. It is now `src/openhuman/memory/preferences.rs`.

**The move needed no new contract surface**, which is worth recording because
the reflex was to add a `recall_relevant_by_vector` method. The engine's
version was itself a *default* method over `query_namespace_hits` — the query
the contract already exposes as `MemoryRetrieval::recall_namespace_scored` — so
the filter (keep hits whose `score_breakdown.vector_similarity` clears the
floor) is reproduced host-side verbatim. Check for a default implementation
before widening the contract; twice now the surface was already there.

A driver without `Capability::Retrieval` yields **no** preferences rather than
an error, preserving the engine default that let keyword-only backends opt out.
Both callers degrade correctly: an absent Lane-B block and an absent
contradiction check, rather than a failed chat turn or a failed preference
write.

The two `KwEmbedder`-based contradiction tests were deleted, not ported. They
existed to make vector similarity move at all through a real `UnifiedMemory`;
the new tests script the score breakdown directly, which pins the similarity
gate the embedder was only an indirect way of reaching.

### 2u. Two reusable test providers now exist

Conversions kept costing test coverage, so `memory/guard/in_memory.rs` now
carries two, both `#[doc(hidden)] pub` rather than `#[cfg(test)]` so integration
tests under `tests/` can see them:

- **`InMemoryProvider`** — real storage; for round trips. `recall` substring-matches.
- **`FixedRecallProvider`** — `recall` answers a scripted list whatever the
  query; everything else inert. For the channel/context tests that assert on
  what the *caller* does with a result set (scoring filter, truncation, budget),
  where recall itself must be a constant.

`guard_over(provider)` wraps either — or a test's own provider — in a real
`MemoryGuard`, so these run through the same policy decorator production uses.

**Where a fake is not enough.** A test whose assertion turns on *ranked* recall
cannot use either, and gets parked on `OPENHUMAN_MODULE_PATH` with the existing
reason string. Three joined that set here: the two `tool_stats` tests, the
`save_preference` storage tests, and the channels autosave test (which asserts
an autosaved turn is later recalled by a differently-worded question — ranking,
not substring).

### 2v. A pre-existing debug-stack overflow in the whole-lib run

`agent::harness::session::runtime::tests::run_single_publishes_completed_and_error_events`
aborts the **entire** `cargo test --lib` run with a stack overflow. It is deep
frames, not recursion: it passes under `RUST_MIN_STACK=16777216`.

**It is not this port's.** Verified by building the branch's merge-base with
`main` (`c5d5eaab6`) in a scratch worktree and running the single test there —
same overflow, same abort. Reverting this port's two edits to the turn body did
not change it either.

It matters here for one practical reason: because the abort kills the process,
**a whole-lib run reports nothing at all** — no counts, no failure list. Every
verification in this port is therefore module-scoped, and a claim of "no new
failures" rests on comparing per-module failing sets against the recorded
baseline, not on a green whole-suite run. Anyone re-checking this work should
know that the suite cannot currently be run end to end, and that this is true
of `main` as well.

Worth knowing separately when adding an `await` to the turn body, since it is
already close to the edge: `situational_preferences` and `standing_preferences`
are free functions rather than inline blocks so their state machines stay off
the caller's frame.

A second pre-existing failure surfaced by the same sweep, verified the same way
against `c5d5eaab6`:
`agent::harness::session::turn::tests::turn_triggers_configured_memory_agent_before_parent_prompt`
asserts the parent turn answers `"parent final"` and gets the *memory agent's*
scripted reply instead. Only one model call reaches the test's `SequenceProvider`,
because `run_subagent` builds the memory agent its own model from config rather
than inheriting the parent's — so the subagent never consumes response #0 and
the parent does. Identical on the merge-base; not this port's, and not fixed
here.

### 2w. Twelve more `install_for_tests` order-dependence defects

The module-by-module sweep found twelve tests that build an agent or a memory
client without calling `host_impls::install_for_tests()` — nine in
`integrations::composio::ops_tests`, three in `cron::scheduler_tests`. Each
fails with *"no EmbeddingHost installed"* when its module is run on its own and
passes in a bigger run, because the installer is `Once`-guarded and some earlier
test happened to call it.

That brings this port's total to **fifteen** (after `agent::learning::startup`,
`sync_pipeline_e2e_tests` and `flows::ops`). The pattern is consistent enough to
state as a rule: **a test that builds an agent, a memory client or a cron job
must install the seam itself.** Relying on a sibling makes the test's own
scoped run a false negative, which is exactly how these survived — nobody runs
`cargo test --lib openhuman::cron` in CI, and the whole-lib run aborts (§2v)
before the counts print.

### 2x. The raw SQLite connection is gone — `Episodic`, the 18th family

The archivist post-turn hook held the last live `rusqlite::Connection` in the
host, handed to it straight out of the session factory. A connection cannot
cross a bus, so this was the hard half of the blocker: while it existed, the
engine could not leave regardless of what happened to the store selector.

It turned out to be much less entangled than "a raw connection" suggests. The
hook issued **no ad-hoc SQL** and knew nothing of the schema; it called ten
typed free functions, and **two of those took no connection at all**. So the
split was already there, waiting to be named:

| Went to the contract (`Capability::Episodic`) | Stayed host-side (`archivist::boundary`) |
| --- | --- |
| `insert_turn`, `session_turns` | `detect_boundary` + its `BoundaryConfig` / `BoundaryDecision` / `BoundaryReason` |
| `open_segment`, `create_segment`, `append_turn` | `incremental_mean_embedding` |
| `close_segment`, `set_segment_summary`, `upsert_segment_embedding` | `fallback_summary` |

Persisting a segment is storage. Deciding *that a segment should end* — how long
a pause means the subject moved on, how many turns is too many, which phrases
announce a change of topic — is a product judgement about what a conversation
is, and the host that renders these segments is the only thing that can tune it
against what users see. The thresholds are carried over verbatim: this is a
move, not a retune, so a regression stays attributable.

**`insert_turn` returns the id, and that is a bug fix, not just a round trip
saved.** The old code inserted a row and then asked `SELECT last_insert_rowid()`
on the same connection. That is *connection-local* state: any interleaved insert
from another task yields the wrong id and files the turn under the wrong
segment. Returning the id from the insert removes the race and the second hop
together.

`ConversationSegment` carries `embedding` for the same reason the family exists
at all — boundary detection is host policy but reads the driver's centroid, so
it comes back on the read rather than costing a second call.

Version: **(2, 1) → (2, 2)**, a minor bump, per the rule that a new family is
made safe by capability negotiation alone.

Both contract copies, the guard decorator (`GuardedEpisodic`), the
`RecordingProvider` fake and the four count pins moved together. The count pins
are worth keeping literal: each one forced a deliberate look rather than sliding
past, which is exactly what caught `every_capability_family_is_accounted_for_in_the_rpc_surface`
needing an entry.

### 2y. The module driver ignores the workspace it is bound for

Found while sizing the store-selector half, and it is worth stating plainly
because it changes what that work is:

`binding::for_workspace` caches on `(workspace_dir, cfg)` and `build()` logs
`workspace=…`, but `module_provider(_workspace_dir)` **discards the argument**.
`ModuleMemoryProvider` resolves against the `Config` published once at boot by
`set_modules_policy`, and reaches a single object path on the bus. So today the
module serves exactly **one store per process** — not one per workspace, and
certainly not one per profile subtree. Two workspaces get two `MemoryBinding`s
that talk to the same store.

That means the `dedicated_memory` question is not "how do we keep the existing
per-subtree behaviour through the bus" — there is no per-subtree behaviour on
the module path to keep.

### 2z. `OpenStore` — the module opens stores, so the contract does not change

Two candidates presented themselves first, and both were wrong:

- **A store selector on the wire** — the object is a singleton by construction,
  so selecting a store means a parameter on *every method of all 18 families*: a
  major contract bump, to express something that is not a property of a memory
  operation at all.
- **Profile subtrees become namespaces** — no contract change, but it relocates
  user data on disk and needs a migration.

The third dissolves the problem. **Which store you are talking to is settled
when you are handed a driver**, exactly like which workspace you are bound to —
it was never a per-call fact. tinybus already supports `serve_at` on many paths,
so the module's root object gained one method:

```text
OpenStore(memory_subdir) -> object_path
```

Each opened store is an ordinary `MemoryService` exporting the identical
interface. `MemoryProvider` still describes one store; a proxy still talks to
one store. **No contract change, no migration, and paths on disk are exactly
where they already were.**

What landed:

| Side | Change |
| --- | --- |
| `tinymemory-core` | `create_memory_client_in_subdir` — the existing client factory hardcoded `"memory"` |
| `tinymemory-module` | `StoreOpener`, `OpenStore`, per-subtree object paths, `MemoryService::root` vs `::new` |
| host | `ModuleMemoryProvider::in_subdir` + lazy `OpenStore` resolution; `binding::for_subtree`; the cache key gained the subtree |

Four decisions worth keeping:

- **Only the root object opens stores.** An opened store has no `StoreOpener`,
  so the recursion is finite by construction rather than by a depth check.
- **Idempotent per subtree, and recorded only after `serve_at` succeeds.** Two
  live handles to one SQLite file is not hypothetical — the engine migrates on
  open, and concurrent migrations on one file corrupt it invisibly. Caching the
  path before the serve succeeded would strand callers on a path nothing
  answers.
- **The object path is derived and character-checked, never free-form.** A
  subdir arrives from a profile id; an id that fails validation must produce a
  refusal, not a malformed bus path. The rejection message does not echo it —
  it is user data.
- **`in_subdir("memory")` is `None`.** Callers pass whatever
  `memory_subdir_for_suffix` produced without special-casing the shared tree,
  and the shared tree costs nothing extra because the root object is served
  eagerly at setup.

This also fixes the workspace-ignoring bug above for the axis that matters: the
workspace still comes from the boot policy (the module is loaded once per
process and captures it at setup), but the **subtree** is now per binding.

### Still open in stage 2

| File | Why it is not converted |
| --- | --- |
| `query/backend.rs` + its three tool wrappers (`drill_down`, `fetch_leaves`, `query_source`) | **Needs contract surface that does not exist.** `backend::query_source(config, source_id, source_kind, time_window_days, query, limit) -> QueryResponse` has a different shape from `MemoryTree::query_source`, and `fetch_leaves` has no equivalent at all. Three more `MemoryRetrieval` methods, or a widened `MemoryTree` — the latter would be a **major** contract bump. |
| `tools/people.rs` + `people/rpc.rs` | The `People` family exists, but the RPC layer is the real call site and its `people_list` payload carries `interaction_count`, which `RankedPerson` does not. Either the contract gains that field or the RPC wire shape changes; `schemas_tests` pins the current one. |
| `tools/diff.rs` | Uses `tinymemory_core::sources::{get_source, list_sources}` — the source *registry*, which is host-layer config, not engine storage. Belongs in stage 4, not behind the driver. |
| `tools/raw_store/kinds.rs` | `MemoryKind` is the engine's **storage-shape** catalog (raw / chunk / entity / tree / vector / kv / contact) and is unrelated to the contract's `MemoryItemKind`. The tool is a static enumeration with no database access, so there is nothing to route — the question is whether the contract should expose engine storage shapes at all, or whether the tool should go. Needs a decision. |
| `tools/search/hybrid_search.rs` | Uses `UnifiedMemory` + `MemoryItemKind` + `tinycortex::WeightProfile` — a whole retrieval facade rather than a single call. Largest remaining conversion. |
| `query/ingest_document.rs`, `query/query_source.rs` | Type-only imports (`SourceKind`, `SourceRef`) plus test-only direct engine calls; trivial once `backend.rs` is resolved. |

**Stage 3 — the 98 `tinycortex::memory` references.**
56 sit outside `memory/` (`agent/`, `threads/`, `subconscious/`, `channels/`,
`security/`), mostly `tinycortex::memory::conversations`. Route through the
provider or through a host-owned conversation store.

**Stage 4 — bring host-layer code home, and drop `tinymemory-core`.**
Move `sync`, `composio_host`, `chat`, `learning_candidate`, `nlp_host` out of
`tinymemory-core` into `memory/`. Delete `host_impls.rs` in favour of the bus
services in `modules/memory_host.rs`.

**Stage 5 — retire `tinymemory-api`.**
Only reachable once stage 4 lands, per the ordering constraint above. Re-point
the 46 `tinymemory_api::host` references at the host-local `memory::api::host`
and reconcile the 6 diverged files. Touches `config/schema/*`, `inference/`,
`cron/scheduler_gate`, `integrations/composio`. These config types are persisted
serde — field names, defaults and `#[serde(...)]` attributes must not move.
Drop the crate cross-check test in `memory/api/host/embeddings.rs`; the golden
test beside it is what carries the format guarantee afterwards.

**Stage 6 — drop the deps and ratchet.**
Remove all five entries from `Cargo.toml`, forward the gate to
`app/src-tauri/Cargo.toml`, and re-baseline `scripts/kernel-floor.limits` —
`libsqlite3-sys` should leave the kernel profile with the engine.

## 5. Verification

- Both-ways gate tests in `src/core/all_tests.rs` for any new feature gating.
- A regression test per stage, failing before and passing after.
- `scripts/check-kernel-floor.sh` re-baselined only at stage 6, and the shed
  written back — an unratcheted improvement grows back unnoticed.
- Prove each claimed shed with `scripts/assert-shed.sh`, never `cargo tree -i`.

# openhuman#5560 — what is left, and how to finish it

**Handoff document.** Written 2026-08-26 for someone picking this up with no
prior context. Everything here was verified against the tree rather than
inherited from the issue text, because a lot of the issue text and several
in-code comments turned out to be stale — that is the single most important
thing to know before starting.

---

## 1. What the issue asks for

Two acceptance criteria:

1. **No direct engine calls** — `grep -r 'tinymemory_core::' src/` returns nothing
   outside `modules/memory.rs` and `memory/binding.rs`.
2. **Crate out of the build** — `cargo tree -e normal --no-default-features
   --features "$(bash scripts/ci/product-features.sh)"` no longer lists
   `tinymemory-core`.

Criterion 1 is nearly done. **Criterion 2 is not reachable yet**, and section 4
explains exactly what blocks it. That is the headline finding of this work.

**Which count decides criterion 1.** The literal `grep` above and the 37
references section 4 reports are not the same number, and only one of them is
the gate. The `grep` is a raw hit count: it includes doc comments that merely
name the crate, `#[cfg(test)]` and dev-only references, and the retained
`pub use tinymemory_core::…` re-export shims that exist precisely so call sites
elsewhere do not name the engine. **Criterion 1 is met when no *production*
call site outside `modules/memory.rs` and `memory/binding.rs` reaches the engine
directly** — shims and test-only references do not count against it, because
neither is a caller. Section 4's table is the authoritative classification;
`memory/direct_engine_refs_tests.rs` is the ratchet that keeps the production
number honest. Read the table, not the raw `grep`.

---

## 2. State of play

### Branches and PRs

| What | Where | State |
| --- | --- | --- |
| Host work, phase 1 | openhuman PR **#5725**, branch `feat/5560-queue-and-recall-through-the-contract` | **merged** |
| Host work, phase 2 | branch `feat/5560-close-the-last-six` (was stacked on #5725) | **this work** |
| `BootstrapConnection` | tinymemory **#105** | merged, released **v1.11.0** |
| `IsToolkitSyncable` | tinymemory **#106** | merged, released **v1.12.0** |

The host is pinned to **tinymemory v1.12.0** across **six** concrete locations,
in five logical groups: the `vendor/tinymemory` gitlink, `modules/registry.rs`
(release tag + per-platform digests), `modules/memory.rs` (the assumed
capability set), and the three CI workflow files that install the module for
tests (`ci-lite.yml`, `ci-full.yml`, `e2e-reusable.yml`) — which are one group
but three files. Bump all six; missing a workflow leaves CI installing a
different module than the host pins, and that mismatch only shows up as a
runtime `Unsupported`.

### What phase 2 did

Six production files that named `tinymemory_core` directly now go through the
contract. Each was verified individually; none of them depends on the shim work
in section 4.

| File | Was | Now |
| --- | --- | --- |
| `memory/read_rpc/admin.rs` | `tree_source::get_or_create_source_tree` | driver `flush_source_tree` |
| `memory/sync/sync_status/rpc.rs` | `tinycortex::memory_config_from` | driver `sync_statuses` |
| `config/migration_helpers/core.rs` | `store::create_memory_for_migration` | `DriverMemory::for_config` |
| `memory/tree/tree/rpc.rs` | `ingest_pipeline::ingest_email` | `email_items` + driver `ingest_email` |
| `agent/harness/session/builder/factory.rs` | `create_session_memory_with_local_ai` | `DriverMemory::for_subtree` |
| `memory/sync/composio/providers/context_ext.rs` | `ProviderContext` extension trait | **file deleted**; `bus.rs` on `BootstrapConnection` + `IsToolkitSyncable` |

Their allowlist entries were **removed** from
`memory/direct_engine_refs_tests.rs` rather than re-worded, so the ratchet count
is real.

---

## 3. Things that are stale or wrong — read this before trusting any comment

This work repeatedly found documentation that no longer matched the code. Assume
nothing; check.

- **The ratchet's reasons.** Several `ALLOWED` entries cited symbols that do not
  exist in the tree any more (`store::fts5`, `store::segments`,
  `approx_token_count`). Those reasons are what routed a whole phase of work
  upstream that did not need to go there.
- **The ratchet does not resolve `#[cfg(test)]`.** It scans files on disk and
  skips comment lines only. Of the sixteen entries once marked
  `NeedsWiderSeam`, **eight were test-only** (mostly via a gated `mod`
  declaration in a *parent* module) and **two were a dev binary**
  (`library-profile`, `required-features = ["rss-bench"]`). Only six were real.
- **`migration_helpers/core.rs` claimed an upstream ask was needed** for a
  capture-budget-exempt store. The budget is enforced *host-side* in
  `memory/guard/policy.rs`, and `DriverMemory` wraps `binding.provider()` — the
  **unguarded** driver — so no budget applies. No upstream change was required.
- **`1bf2037a0` removed the memory host seams** on the reasoning that "this
  process embeds no engine". It does. That shipped a build where **every chat
  turn failed** with `no EmbeddingHost installed`. Restored in `00fc1fba2`.
  See section 6.
- **The kernel-floor entry (2026-08-10) is probably stale.** It ratchets
  `tinymemory` / `tinymemory-tinycortex` in as always-on "driver admission"
  dependencies. Measured: the product build compiles **clean** without both, no
  `ctor`/`inventory`/`linkme` registration exists in either, nothing in the
  product build names them, and `binding::admit` refuses `DriverClass::Embedded`
  outright. **That evidence is not sufficient on its own, and the entry stays
  until a process-level smoke test says otherwise.** `admit` maps both the
  default `tinymemory` driver and the legacy `tinycortex` alias to
  `DriverClass::Module`, and the provider then loads the module through
  `ops::ensure_loaded` — a `dlopen` at runtime. A clean compile and a host-side
  source scan cannot observe that path, so they cannot prove removing the deps
  leaves module admission working. Run a build with them dropped and drive one
  real module-backed memory call before rewriting the entry. Either it gets
  rewritten on that evidence or the deps stay — leaving two
  documented decisions contradicting each other is how the next person is misled.

---

## 4. Why criterion 2 is blocked

`tinymemory-core` is reached two ways:

```text
tinymemory-core
├── openhuman                 ← via 13 `pub use tinymemory_core::…::*` shims
└── tinymemory-tinycortex     ← transitive
    └── openhuman
```

**The second parent is solvable.** Removing `tinymemory` and
`tinymemory-tinycortex` from `Cargo.toml` leaves the product build compiling with
zero errors, and `tinymemory-core` then has `openhuman` as its **only** parent.
That experiment was run; re-run it to confirm before relying on it.

**The first parent is not, yet — and this is the real blocker.** The shims cannot
simply be deleted. `memory/tree/mod.rs` says so in its own docs:

> None of it has a contract equivalent: `tinymemory_api::tree` is the summary
> node vocabulary, not an embedder or an entity extractor. So this shim is
> pinned by the engine's *scoring and summarisation* internals, and it goes when
> those move behind the bus — **not before**.

Verified: `tinymemory-api` provides **zero** files for `score`, `summarise`,
`nlp` or `ingest`. Deleting the shim would force consumers to write
`tinymemory_core::tree::score::embed` directly — *more* direct engine references,
not fewer. Strictly worse.

### The actual remaining surface

**37 references across ~20 files.** Not the 214 a naive grep suggests — most of
that count is host code, because `health`, `retrieval`, `tree` and
`tree_runtime` are **shadowed** by local `pub mod` declarations and only look
like engine paths.

| Cluster | Refs | Contract family exists? | Verdict |
| --- | --- | --- | --- |
| `sources::{types, registry, reconcile, status, readers, sync, get_source, list_sources, apply_kind_defaults, …}` | ~15 | **`as_sources` / `MemorySourceSink` exists** | probably routable **today** — verify member-by-member |
| `diff::{types, ops}` | 5 | **`as_diff` / `MemoryDiff` exists** (`capture_snapshot`, `snapshots`, `diff`) | probably routable **today** — verify |
| `tree::{score, summarise, nlp}` | 9 | **no equivalent** | **needs upstream** |
| `tree::{store, runtime}` | 5 | unknown | check first |

So the work splits into "probably already possible" (~20 refs) and "needs a real
upstream program" (~9 refs).

### The upstream program

`score::{embed, extract, store, resolver}`, `summarise::{summarise,
fallback_summary, SummaryContext, SummaryInput}`, `nlp::extract_query_entities`.
That is embedder construction, summarisation and entity extraction — a
materially bigger contract surface than either member shipped for this issue
(#105, #106 were one method each).

Consumers: `agent/harness/archivist/{lifecycle,recap,types,test_constructors}.rs`,
`agent/harness/subagent_runner/ops/runner.rs`, `inference/embeddings/rpc.rs`,
`memory/read_rpc/entities.rs`, `memory/tree/retrieval/rpc.rs`.

**Design question to settle first, before writing any code:** does the host
*call* an embedder, or does it *ask the driver to embed*? The module already
proxies embedding back to the host through `EmbeddingHost` — so a naive
"expose the embedder over the bus" would be circular. The likely right shape is
that scoring and summarisation become driver-side operations the host requests
by intent, not primitives the host drives. Get that wrong and the seam gets
wider without getting cleaner.

---

## 5. Recommended sequence

Each step is independently valuable and independently mergeable.

1. **Land the draft PR** (this work). Criterion 1 progress, no dependencies.
2. **Fix the ratchet's measurement.** Teach the scanner to resolve
   `#[cfg(test)]` — including a gated `mod` declaration in an ancestor — and to
   recognise `required-features` binaries. Report test-only refs as a separate,
   still-visible count. Re-verify every remaining reason against the code and
   delete the stale ones. *Do this before anything else: its wrong reasons are
   what sent a phase of work upstream unnecessarily.*
3. **Route `sources` and `diff`.** ~20 refs against families the contract
   already has. Verify member-by-member that the contract covers what each call
   site needs; where it does not, note the gap rather than forcing it.
4. **Scope the scoring/summarisation contract** as its own issue, starting from
   the design question above. Do not start with code.
5. **After 4 ships and releases:** delete the 13 shims, drop `tinymemory`,
   `tinymemory-tinycortex` and `tinymemory-core`, remove the
   `[package.metadata.cargo-machete] ignored` entries, ratchet the kernel floor
   down, and reconcile or delete the 2026-08-10 kernel-floor entry.

---

## 6. Hazards that have already caused outages

Read these as "this has happened", not "this could happen".

- **A `cdylib` has its own statics.** The module cannot see a process-global the
  host filled, and vice versa. This produced *two* separate runtime failures in
  one day: the missing `EmbeddingHost` (every chat turn failed) and an empty
  Composio provider registry inside the module (every bootstrap would have
  reported "no composio provider registered"). **Neither is visible to
  `cargo check`, to the ratchets, or to any unit test** — they all run in a
  process that is already initialised.
- **Never remove a seam before its last caller.** The seams are downstream of the
  callers, not an independent item to trim. Prove the caller is gone, then
  remove the seam.
- **Compile-clean is not sufficient here.** Boot the app and send a chat turn
  before merging anything that changes memory wiring.
- **The release-pin gate is five sites, and they move together:** the
  `vendor/tinymemory` gitlink, `modules/registry.rs` (version, release URL, 11
  archives, 11 digests), `ARTIFACT_CAPABILITIES_PIN` in `modules/memory.rs`, and
  the `memory_version` / `memory_sha256` pairs in `ci-full.yml`, `ci-lite.yml`
  and `e2e-reusable.yml`. Take digests **verbatim from the release's
  `checksum.toml`**, never recomputed locally.
- **`is_compatible` has zero call sites.** A member inside an already-advertised
  family that the pinned artifact does not serve fails at **runtime** with
  `UnknownMethod`, not at build. Before trusting a re-pin, diff the host's
  `module_call!` member names against the tag's `METHODS` list. It has been
  64/64 at every pin so far.

---

## 7. Verification commands

```bash
# both clippy lanes CI runs — the second catches gates-off breakage
cargo clippy -p openhuman --features "$(bash scripts/ci/product-features.sh)" -- -D warnings
cargo clippy -p openhuman -- -D warnings

# tests/ targets are NOT built by PR CI (test.yml is workflow_dispatch-only),
# so this is the only place they get checked before a release lane sees them
cargo check --locked -p openhuman --features "$(bash scripts/ci/product-features.sh)" --tests

cargo test -p openhuman --lib -- memory:: modules::      # multi-filter needs `-- a b`

# Two Cargo worlds, two lockfiles — the root command does NOT validate the
# shell's. Check both, or a stale `app/src-tauri/Cargo.lock` fails the release
# lane in "Enforce Linux TLS dependency policy" with a --locked error that
# names neither the lockfile nor the pin that moved.
cargo metadata --locked
cargo metadata --locked --manifest-path app/src-tauri/Cargo.toml

# criterion 2, the real gate. NOT `cargo tree -i`: it exits non-zero when the
# crate is absent (so "gone" and "command failed" are the same signal) and
# `-e normal` still matches a dev-dependency-only survivor.
bash scripts/assert-shed.sh "$(bash scripts/ci/product-features.sh)" tinymemory-core
```

macOS notes: set `GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=commit.gpgsign
GIT_CONFIG_VALUE_0=false` or temp-repo tests hang forever on a pinentry prompt;
`git commit`/`merge` want `-c core.hooksPath=/dev/null` or they stall; system
bash is 3.2 so CI scripts using `mapfile` cannot be run locally as-is.

---

## 8. Known-red, not yours

- **`tinysweeper/commits` on #5725.** Three findings, all false positives: two
  are the regex *detectors* in `memory/safety.rs` that redact real OpenSSH and
  PGP keys, one is a `MIIabc123` test fixture. Removing any deletes real
  redaction or the test proving it fires. Needs a maintainer dismissal. The bot
  is also degraded (`402 Payment Required` on its embeddings), so four of its
  six lanes reported "no reviewer could be consulted" or reviewed 0 files.
- **`subagent_prompt_renderer_handles_formats_caps_and_stale_tool_indices`** on
  `main` asserts `"## Output style"`, a heading that moved to `STYLE.md` in
  #5701. Pre-existing; surfaces only if the full suite runs.

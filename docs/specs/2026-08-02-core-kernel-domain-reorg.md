# Core kernelization, part 2 — the domain-family reorg

**Status:** structural half complete · **Date:** 2026-08-02 · **Scope:** `src/openhuman/**`
**Companions:** `docs/specs/kernel.md` — the subsystem/driver model this feeds into, still an
uncommitted design draft, so it is referenced by name rather than linked ·
[`../plans/pluggable-core/README.md`](../plans/pluggable-core/README.md) (the host-side
`CoreBuilder`/`CoreContext` work) · `AGENTS.md` § *Compile-time domain gates*

---

## 1. Where the program stands

The kernelization program has two halves. **The dependency half is largely done**; the
**structural half is complete** — 124 top-level domain directories are down to 31, and both
root-level `*.rs` violations are gone.

### Done (#4795 epic, then #5314)

`#5314` took the `--no-default-features --features flows` profile from **418 → 280** unique crate
names by gating dependency cohorts, and — more importantly — **built the ruler**:

| Tool | What it does | CI-wired? |
| --- | --- | --- |
| `scripts/kernel-floor.sh` | Counts packages / unique names / native-toolchain builds for a profile | via the checker |
| `scripts/kernel-floor.limits` | The ratchet. Only goes down; raising a number needs written justification | yes |
| `scripts/check-kernel-floor.sh` | Fails CI when a profile exceeds its limit | `ci-lite.yml:460` |
| `scripts/assert-shed.sh` | Proves a crate is absent from the **normal** edge graph | manual |
| `scripts/ci/check-feature-forwarding.mjs` | Fails when a default-ON gate is not forwarded to the desktop shell | yes |

> **`assert-shed.sh`, not `cargo tree -i`.** The older checklists (including the first draft of
> this plan) said to prove a shed with `cargo tree -i <crate>`. That is wrong twice over: it
> *exits non-zero* when the crate is absent, so a naive `if` inverts the result; and `-i` resolves
> against the whole package set, so a crate that survives only as a dev-dependency is reported as
> present. Use `scripts/assert-shed.sh <profile> <crate>...`.

Current floor, measured 2026-08-02: **312 packages / 285 names / 6 native**
(`aws-lc-sys`, `libgit2-sys`, `libsqlite3-sys`, `libz-sys`, `lzma-sys`, `ring`).
Stated target: **222 names / 2 native** (`libsqlite3-sys`, `ring`).

The 312/285 line is above the 280 low-water mark because merging `origin/main` inherited +5 names
from unrelated upstream work (#5232 SDK vendoring, #5202/#5233 backend-client refactors). All
seventeen previously-shed crates were re-asserted absent. Do not raise it again without re-running
that assertion — a merge silently re-adding a shed crate looks identical to upstream growth if you
only compare totals.

### In progress — the structural half

At the start of this work `src/openhuman/` was **124 flat directories** plus two root-level
`*.rs` files (`util.rs`, `dev_paths.rs`) violating the AGENTS.md "no new root-level `*.rs`" rule,
with only **12** carrying a module-level `#[cfg]`. A single capability was spread across sibling
top-level dirs — `memory*` is 13, `agent*` is 6, `mcp_*` is 4, `runtime_*` was 3 — so every gate
meant `#[cfg]`-ing scattered `pub mod` lines and hand-syncing five parallel registries
(`DomainGroup`, `DomainSet`, `StoreInitPlan`, `DomainSubscriberPlan`, `tool_group()`).

**Steps 1–9 landed: 124 → 31 directories, 2 → 0 root-level `*.rs`.** Families so far: `meet/`,
`util/`, `sandbox/cwd_jail`, `cron/scheduler_gate`, `runtime/`, `media/`, `desktop/`, `hosted/`,
`subconscious/{triggers,monitors}`, and `mcp/`, and the existing-gate families `voice/audio_toolkit`,
`web3/{wallet,x402}`, `medulla/chat`, `flows/{tinyflows,rhai}`,
`channels/{whatsapp_data,webview_accounts}`, and step 6's seven medium families —
`threads/{goals,todos}`, `tools/{registry,status,timeout,agent_policy}`, the new `platform/`
(ten host-platform domains), `config/{migrations,migration_helpers,workspace}`,
`integrations/{composio,recall_calendar,file_storage,task_sources}`,
`skills/{catalog,runtime,webhooks}`, `inference/{embeddings,tokenjuice}`, and step 7's kernel
`security/{approval,credentials,keyring,keyring_consent,encryption,prompt_injection,devices}`.
The `heartbeat/` re-export shim is deleted, step 8 folded fourteen domains into `agent/`, and
step 9 folded thirteen domains into `memory/` — the last and largest family.

**That is what this document is about.** The remaining dependency sheds are blocked on it or are
cross-repo; the structural work is what makes the next twenty gates cheap instead of expensive.

---

## 2. The organizing rule

> **A family directory exists if and only if its members would be compiled out together.**

Family boundary == future gate boundary. Directories whose names merely rhyme do not merge
(`orchestration` vs `agent_orchestration`; `web_chat` is **not** part of `channels`).

Kernel vs subsystem is decided by the `kernel.md` §4 criterion: *would a build whose only driver is
a third-party external backend still need this file?* Yes → kernel. No → subsystem.

### Why the move is safe

RPC namespaces are **string literals in `ControllerSchema`**, not derived from module paths. So
the `/rpc` surface, `/schema` output, agent-tool names, `DomainEvent::domain()`, and the frontend
catalog are **byte-identical after a move**. All `include_str!`/`include_bytes!` sites and all
`#[path = "…"]` attributes are intra-directory and travel with their directory. There are zero
`module_path!()` call sites in `src/`.

Corollary, and the pilot proved it: after moving `agent_meetings` → `meet/backend_bot`, the
namespace string is still `"agent_meetings"`. **Do not "fix" namespace strings to match new
paths** — directory layout and wire surface are independent axes.

### Rules for a move PR

- One PR per family. `git mv` + a mechanical path rewrite. No logic change, no behaviour change.
  If a hunk isn't a path, it doesn't belong in the PR.
- **No `pub use` transition shims.** The compiler catches 100% of intra-crate breakage.
  `src/openhuman/heartbeat/` was the standing counter-example: a 10-line shim added "so external
  paths keep compiling without a crate-wide rename", which then sat there indefinitely with live
  call sites. It had three real callers; step 3 retargeted them and deleted it. Worse than
  useless, a shim is an always-compiled `pub mod` re-exporting into a gated tree, which defeats
  the gate it fronts.
- Do not touch `DomainGroup`/`DomainSet`. That realignment is Phase 5 and needs its own tests.
- **A `#[cfg]` may move, but the compiled set may not change.** Nesting a facade+stub domain under
  a leaf-gated parent forces the parent to become a facade (see the pilot). That is a mechanical
  consequence, not a behaviour change, and the both-ways tests prove it.

---

## 3. Pilot — `meet/` (landed)

```
src/openhuman/meet_agent/     -> src/openhuman/meet/agent/
src/openhuman/agent_meetings/ -> src/openhuman/meet/backend_bot/
```

Chosen as the pilot because it is the smallest family that already has a gate *and* exercises all
three module patterns at once: `meet` was leaf-gated, `meet/agent` is leaf-gated internally, and
`meet/backend_bot` is facade+stub with three always-compiled callers.

It surfaced the one non-obvious rule above: `pub mod meet;` had to become **ungated**, with the
`#[cfg(feature = "meet")]` pushed down onto each submodule in `meet/mod.rs`, because
`backend_bot`'s stub must resolve in a `meet`-less build. Same set of items compiles either way.

Verification that a family-move PR must reproduce:

```bash
GGML_NATIVE=OFF cargo check --all-targets
GGML_NATIVE=OFF cargo check --lib --no-default-features
GGML_NATIVE=OFF cargo check --manifest-path app/src-tauri/Cargo.toml
GGML_NATIVE=OFF cargo test --lib core::                                             # gates on
GGML_NATIVE=OFF cargo test --lib --no-default-features core::                       # gates off
cargo fmt --check
bash scripts/check-kernel-floor.sh --verbose      # must not move
node scripts/ci/check-feature-forwarding.mjs
node scripts/generate-test-inventory.mjs --check  # keyed on namespaces, so it should pass unchanged
```

Expect `cargo fmt` fallout: longer paths re-wrap imports. That is the whole diff outside the
renames.

### Gotchas the first two moves hit

- **Braced imports.** `use crate::openhuman::{scheduler_gate, todos};` is invisible to a
  `s/openhuman::scheduler_gate/…/` rewrite. There are only a handful crate-wide; the compiler
  finds them immediately, but budget for a hand-fix per family.
- **The rewrite loop must not word-split in zsh.** `sed -i … $files` passes the whole newline-
  separated list as *one* argument under zsh (which does not word-split unquoted parameters), so
  the rewrite silently does nothing and the greps still show old paths. Use
  `grep -rlZ … | xargs -0 sed -i …`.
- **One observable string DID change, deliberately.** The tracing target in
  `tools/agent_policy/engine.rs` went from `"openhuman::agent_tool_policy"` to
  `"openhuman::tools::agent_policy"` (6 sites). Tracing targets mirror module paths by
  convention — the other three `target: "openhuman::…"` literals in the crate all do — so
  leaving the old value would have pointed a log filter at a module that no longer exists.
  Nothing operator-facing referenced it (no docs, scripts, or CI). Recorded here because a
  pure-move commit must never change an observable string *silently*; `RUST_LOG=openhuman::
  agent_tool_policy=debug` now matches nothing, and tracing's EnvFilter fails open.
- **Pre-existing failures to not chase.** `cron::scheduler::tests::{run_agent_job_returns_error_without_provider_key,
  cron_agent_job_short_loopback_send_error_stays_retryable}` overflow the stack on `main`, before
  any reorg. Verify with `git stash -u` + rerun before assuming a move caused a failure.

---

## 4. Target tree — 124 dirs + 2 root files → ~30 dirs + 0 root files

### Subsystems (gateable families)

| Family | Absorbs |
| --- | --- |
| `memory/` | ✅ **landed** — `memory_store→store`, `memory_sync→sync`, `memory_tree→tree`, `memory_search→search`, `memory_sources→sources`, `memory_queue→queue`, `memory_diff→diff`, `memory_goals→goals`, `memory_conversations→conversations`, `memory_tools→tool_memory`, `tinycortex`, `agent_memory→agent`, `people`; parent stays put (kernel, ungated) — `memory/sync.rs` renamed to `memory/sync_events.rs` first to free the name, and `memory_tools` lands as `tool_memory` to dodge the pre-existing `memory/tools/` |
| `agent/` | ✅ **landed** — `agent_experience→experience`, `agent_orchestration→orchestration`, `agent_registry→registry`, `agentbox`, `harness_init`, `session_db`, `session_import`, `context`, `profiles`, `learning`, `plan_review`, `file_state`, `artifacts`, `tinyagents`; parent stays ungated (kernel) and keeps its own name — no `agent/core` rename |
| `inference/` | ✅ **landed** — `embeddings`, `tokenjuice`; parent stays ungated (kernel). NB the `inference` Cargo feature gates only the cpal capture stack + microphone probe, *not* this directory |
| `skills/` | ✅ **landed** — `skill_registry→catalog` (not `registry` — `skills/registry.rs` and the stub's inner `pub mod registry` both already own that name), `skill_runtime→runtime`, `webhooks`; parent stays ungated (three facades, two with `stub.rs`), and `webhooks` is a permanently-ungated child |
| `flows/` | ✅ **landed** — `tinyflows`, `rhai_workflows→rhai`; parent is leaf-gated on `flows` (no stub — every external site is a registration site) |
| `mcp/` *(new)* | ✅ **landed** — `mcp_server→server`, `mcp_registry→registry`, `mcp_audit→audit`, `mcp_client::{registry,stdio,spawn_env,setup_agent}→config_servers` *(leaf-gated)*, `mcp_client::{client,client_helpers}→http_client` *(ungated carve-out)*, `mcp_client::sanitize→util/sanitize`; parent stays ungated (three facades with `stub.rs` + the always-compiled `http_client`) |
| `channels/` | ✅ **landed** — `whatsapp_data`, `webview_accounts`; parent stays ungated (the `traits`/`cli` carve-outs), gate pushed onto each child |
| `meet/` | ✅ **landed** — `meet_agent→agent`, `agent_meetings→backend_bot` |
| `voice/` | ✅ **landed** — `audio_toolkit`; parent stays ungated (facade + `stub.rs`), gate pushed onto the child |
| `web3/` | ✅ **landed** — `wallet`, `x402`; parent stays ungated (all three are facades with their own `stub.rs`) |
| `media/` *(new)* | ✅ **landed** — `media_generation→generation`, `image`; parent is leaf-gated on `media` since both children were wholly gated |
| `medulla/` | ✅ **landed** — `medulla_chat→chat`; parent stays ungated (facade + `contract`/`events` type carve-out), gate pushed onto the child |
| `runtime/` *(new)* | ✅ **landed** — `runtime_node→node`, `runtime_python→python`, `runtime_python_server→python_server`, `runtime_pool→pool`, `javascript` |
| `integrations/` | ✅ **landed** — `composio`, `recall_calendar`, `file_storage`, `task_sources` (a move into an already-populated parent) |
| `hosted/` *(new)* | ✅ **landed** — `billing`, `referral`, `announcements`, `team`, `orchestration` |
| `desktop/` *(new)* | ✅ **landed** — `accessibility`, `overlay`, `dashboard`, `provider_surfaces`, `notifications`, `app_state` |
| `subconscious/` | ✅ **landed** — `subconscious_triggers→triggers`, `monitor→monitors`, `heartbeat/` shim deleted |
| standalone | `search/`, `tinyplace/`, `web_chat/`, `http_host/`, `test_support/` |
| `json_schema/` | **kernel, deliberately unowned.** Vendor-neutral JSON Schema / JSON value walking, shared by the Composio catalog (`integrations/composio`, always compiled) and the tinyflows capability adapters (`flows/tinyflows`, gated). It belongs to neither: housing it in either would force a dependency edge from the other, and one of those directions is the always-on → gated back-edge the kernelization work exists to remove. Ungated, no gate planned. |

### Kernel (never gated)

| Family | Absorbs |
| --- | --- |
| `config/` | ✅ **landed** — `migrations`, `migration→migration_helpers`, `workspace`; the two migration dirs stay distinct per rule 7 |
| `security/` | ✅ **landed** — `approval`, `credentials`, `keyring`, `keyring_consent`, `encryption`, `prompt_injection`, `devices`; parent and every child stay ungated (kernel). No collision with the pre-existing `security/pairing.rs` |
| `tools/` | ✅ **landed** — `tool_registry→registry`, `tool_status→status`, `tool_timeout→timeout`, `agent_tool_policy→agent_policy` |
| `platform/` *(new)* | ✅ **landed** — `service`, `startup`, `update`, `doctor`, `health`, `proc_metrics`, `connectivity`, `about_app`, `cost`, `socket` |
| `threads/` | ✅ **landed** — `thread_goals→goals`, `todos` |
| `cron/` | ✅ **landed** — `scheduler_gate` |
| `sandbox/` | ✅ **landed** — `cwd_jail` |
| `util/` *(new)* | ✅ **landed** — `util.rs` split into `util/{mod,text,retry,types}.rs`, `tls→util/tls`, `dev_paths.rs` deleted, `mcp_client::sanitize→util/sanitize` (landed with the `mcp/` move) |

### Name collisions — dodge, don't pay

`memory/` already contains `sync.rs` and `tools/`. Renaming `memory → memory/core` costs ~545
import rewrites; instead rename `memory/sync.rs → memory/sync_events.rs` (6 external refs) and
land `memory_tools` as `memory/tool_memory/` (3 refs). Likewise `agent/tool_policy.rs` already
exists, so `agent_tool_policy` goes to `tools/agent_policy/`; `agent/cost.rs` exists, so `cost`
goes to `platform/`.

### Must not move / must not change

1. **`web_chat/` stays top-level** — deliberately decoupled from `channels` in #5002/#5003;
   always-compiled despite its `DomainGroup::Channels` tag; the channels both-ways test asserts
   `channel` survives with the feature OFF.
2. `channels::{traits, cli}` stay ungated carve-outs — reached by the always-on
   `agent::harness::session::runtime::run_interactive`.
3. `skills::{types, ops_types}` stay ungated and stay put — ~236 files consume `ToolResult` /
   `ToolContent` through `tools/traits.rs`. This is the largest import fan-out in the crate.
4. `tools/` ownership rule untouched — domain tools stay in each domain's `tools.rs`, re-exported
   through the globs in `tools/mod.rs`. Only the glob's *path* changes.
5. `mcp::registry::types`, `mcp::audit::types`, `mcp::server::tools::types` stay ungated.
6. `tinyplace/` does **not** go under `web3/` — its signer works via ed25519 independently.
7. Do not merge `migration/` into `migrations/` during a move (pure moves only).
8. **`scripts/ci/orch-ip-gate.sh` hard-codes domain paths and fails *open* on a wrong path.**
   Retargeted to `src/openhuman/hosted/orchestration/…` in step 3; its
   `src/openhuman/subconscious/profiles/tinyplace.rs` reference is still valid because
   `subconscious/` stayed put. Re-check this script on any move that touches either path, and
   confirm the directory it names actually exists — a pass proves nothing on its own.
9. `scripts/agent-batch/` specs use `owned_paths: ["src/openhuman/<dom>/"]` — sweep live specs.

### Order

1. ✅ `meet/` — pilot.
2. ✅ `util/` + `sandbox/` + `cron/` — cleared both root `*.rs` violations and deleted dead
   `dev_paths.rs`. (`sanitize` moves out of `mcp_client` with step 5, not here — a pure move PR
   should not also carve a module out of an unrelated domain.)
3. ✅ `runtime/`, `media/`, `desktop/`, `hosted/`, `subconscious/` — new parents. `hosted/` also
   retargeted `scripts/ci/orch-ip-gate.sh` (see rule 8).
4. ✅ `voice/`, `web3/`, `medulla/`, `flows/`, `channels/` — existing gates; each validates that its
   `stub.rs` survives relocation.
5. ✅ `mcp/` — the only family with a genuine *split*. `mcp_client` divided three ways:
   `config_servers` (leaf-gated), `http_client` (ungated carve-out), `sanitize` → `util/`.
6. ✅ `threads/`, `tools/`, `platform/`, `config/`, `integrations/`, `skills/`, `inference/` — seven independent families, one commit each.
7. ✅ `security/` — the kernel security family; `kernel.md` §3.4's `Guard<D>` draw set made physical.
8. ✅ `agent/` — fourteen domains folded in; `agent/` itself stayed put as the parent (an
   `agent → agent/core` rename would have cost ~999 extra import rewrites and buys no gate).
   Kernel: no `#[cfg]` anywhere in the family.
9. ✅ `memory/` — **last**, deliberately: it is `kernel.md` §5's pilot subsystem, so its
   layout gets drawn with the driver contract in hand rather than guessed.

Rationale for biggest-last: the tooling (rewrite script, check matrix, PR template) gets proven on
fifteen cheap families before being pointed at the two that are ~57% of the total churn.

---

## 5. Remaining dependency sheds

Target is 6 → 2 native builds. Four of the six are addressable, and two of those are unblocked by
the reorg:

| Native crate | Owner | Gate | Status |
| --- | --- | --- | --- |
| `libgit2-sys` (via `git2`) | `memory_store/content/wiki_git` **and** `tinycortex/git-diff` | `memory-git` | **cross-repo** — see below |
| `lzma-sys` (via `xz2`) | *(none — removed)* | — | ✅ **done**, and better than planned: extraction moved into the `tinyruntime` module, so `xz2` left the manifest for **every** configuration rather than only for builds that opted out of `runtime-node` |
| `libz-sys` | shared (`flate2`/`zip`/`git2`) | — | partly falls out of the above |
| `aws-lc-sys` | TLS stack | — | needs a rustls-provider decision, own slice |
| `libsqlite3-sys`, `ring` | kernel | — | **target keeps these** |

Also ready, no native build: `objc2-contacts` (macOS, sole owner `people/address_book.rs`) behind
a `contacts` gate.

### `memory-git` is a cross-repo change

`vendor/tinycortex` is its own submodule (`tinyhumansai/tinycortex`) and the root pins it with
`features = ["git-diff", …]`, so gating the host's `wiki_git` alone sheds nothing — `git2` still
arrives through the crate. The host's `memory_diff` domain also re-exports
`tinycortex::memory::diff::*`, which lives behind that same feature, and has two real
non-registration callers (`subconscious/profiles/memory.rs`, `memory_sources/sync.rs`).

The clean shape, and the vendor crate is already 90% of the way there —
`vendor/tinycortex/src/memory/diff/types.rs` exists and `git2` is confined to `ledger.rs` +
`ledger_helpers.rs`:

1. **tinycortex PR** — carve `memory::diff::{types, source, snapshot, checkpoint, diff}` out from
   behind `git-diff`, leaving only `ledger`/`ledger_helpers`/`DiffEngine` gated. Same "inert types
   stay ungated, only behaviour gates" rule the `skills` and `mcp` gates follow.
2. **openhuman PR** — `memory-git = ["dep:git2", "tinycortex/git-diff"]`; pin tinycortex with
   `default-features = false, features = ["persona", "sync"]`; leaf-gate host `wiki_git`; make
   `memory_diff` facade+stub with `types` ungated so the two callers need no `#[cfg]`.
3. Bump the gitlink; lower `kernel-floor.limits` in the same PR.

Off-state semantics: wiki content is still written to disk, just not versioned; diff/checkpoint
RPCs are unregistered and the `MemoryDiffTool` is absent.

---

## 6. Superseded by #5314 — do not re-do

- A `check-gate-sheds.mjs` that parses `# SHEDS:` comments → **`assert-shed.sh` +
  the `kernel-floor.limits` ratchet already cover this**, and the ratchet is the stronger guard: a
  gate that stops shedding raises the floor and fails CI.
- A binary-size budget lane → **`check-kernel-floor.sh` is wired at `ci-lite.yml:460`** and counts
  crates/native builds, which is a better proxy for an embeddable library than stripped bytes.
- Making `enigo`/`arboard`/`rdev` optional, the Polymarket/ethers cohort, `starship-battery` →
  all landed in #5314.

## 7. Out of scope — do not re-litigate

- **Gating `agent` wholesale** — `agent::harness` has ~448 external references; `tools/`,
  `web_chat`, and `medulla_session` all depend on it. The harness *is* the kernel's execution engine.
- **Gating `tools` wholesale** — `tools::traits` has ~248 external references. Kernel, permanently.
  Only the `tools/impl/*` families gate.
- **Gating `memory` wholesale** — it becomes a subsystem *slot* (`kernel.md` §5), not
  a feature.
- **Dropping `keyring`, `rusqlite`, `tinyagents`, `tinychannels`, `tinycortex`, `tinyplace`** —
  load-bearing across always-on domains. `tinychannels` in particular was addressed by gating
  *providers inside the vendored crate* (`email`, `lark`), not by gating the crate out; repeat
  that shape rather than proposing the crate-level gate again.

---

## 8. Definition of done

1. `src/openhuman/` is ~30 directories, zero root-level `*.rs` besides `mod.rs`.
2. Every family directory maps 1:1 to a gate or is declared kernel in this document.
3. `kernel-floor.limits` reaches 222 names / 2 native.
4. Each gate has both-ways tests in `src/core/all_tests.rs` and `tools/ops_tests.rs`.
5. ✅ **Done.** `DomainGroup` gained seven variants, not four: `Inference`, `Integrations`,
   `Automation` (cron + subconscious), `Runtimes` (runtime + sandbox), `Desktop`, `Hosted`,
   `Relay` (tinyplace). The extra three over the original estimate are `Inference`, `Desktop`
   and `Hosted` — carving those out is what lets `embedded()` stop setting `platform: true`
   just to reach credentials and config. `Platform` now holds only `platform/`, `tools/`,
   `http_host/`, `test_support/`. `DomainSet::kernel()` and `examples/embed_kernel.rs` exist;
   the example runs and demonstrates memory-on / agent-unknown-method.

   The realignment also fixed two defects the flat tree had hidden: `harness()` was dropping
   ten namespaces (including `harness_init`) into `Platform` despite claiming their families,
   and `StoreInitPlan.people` keyed on `Platform` while its controllers moved to `Memory` —
   which would have registered the people RPC surface with no store behind it.
6. Hand off to `kernel.md`'s subsystem registry (`src/core/subsystem/`, `Driver`,
   `Guard`, `subsystems_status`).

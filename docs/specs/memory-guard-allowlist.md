# Memory-guard allowlist

Every place in the tree that still reaches memory **without** going through
`MemoryGuard`, and why. Produced by M4b; consumed by M4c.

Pinned by the ratchet in `src/openhuman/memory/bypass_allowlist_tests.rs`
(M4c), which fails **both ways** — when a new unguarded call site appears
(`no_new_memory_driver_bypasses`), and when an allowlisted one is cleaned up
without being struck from the list (`bypass_allowlist_has_no_stale_entries`).
Two further tests stop the lint rotting into a rubber stamp: the scanner must
find a known bypass, and every needle must still match something.

M4b shipped a provisional, file-keyed version of this guard inside
`memory/ops/guard_tests.rs`. M4c deleted it — two allowlists over one tree must
both be struck on every cleanup, and the one nobody remembers is exactly the
dead-string rot the ratchet exists to prevent.

## Scope

The lint scans the host and vendored `tinymemory-core` sources for nine
patterns, keyed on `(file, pattern)` so the
failure message names the needle that tripped:

| Pattern | What it hands out |
| --- | --- |
| `global::client_if_ready(` | `MemoryClientRef` |
| `.profile_conn(` | raw `Arc<Mutex<rusqlite::Connection>>` (one in-family site) |
| `.profile_store(` | a typed `ProfileStore` — confined, but still unguarded |
| `.get_document(` | `pub(crate)` read-one escape hatch |
| `NullMemoryProvider::new(` | a driver, built outside `binding::for_workspace` |
| `MemoryClient::from_workspace_dir(` | a second engine on the same store |
| `binding::for_workspace(` / `.memory_binding(` | a raw `MemoryBinding` |
| `.unguarded_provider(` | the raw `Arc<dyn MemoryProvider>` off a `MemoryBinding` |

**By-path test files (`*_tests.rs`, `tests.rs`, `test_support/`) are out of
scope.** Driver tests construct drivers — that is what a driver test *is* —
so allowlisting them would add ~25 entries that can never shrink and would
churn on every new test. Inline `#[cfg(test)] mod tests` blocks are *not*
stripped, because brace-tracking Rust with a line scanner is fragile and
getting it wrong silently hides production sites; the four files affected are
allowlisted with a reason saying so. Comment lines are skipped, so doc-comment
references are not mistaken for calls.

`global::init(workspace)` is deliberately **not** scanned. It binds the
workspace; it does not read or write memory, and every call site is a
login / active-user-switch / boot / CLI-entry lifecycle event
(`security/credentials/ops.rs`, `desktop/app_state/ops.rs`,
`core/runtime/context.rs`, `core/memory_cli.rs`, `core/subconscious_cli.rs`,
`memory/ops/documents.rs`'s `memory_init`, `memory/tinycortex/sync.rs`).

## What M4b re-pointed

Four RPC handlers, all in `src/openhuman/memory/ops/`, each of whose contract
twin is a literal one-line delegation to the same host method on the same
store:

| Handler | Contract method | Driver body |
| --- | --- | --- |
| `documents::doc_put` | `MemoryDocuments::put_document` | `client.put_doc(input)` |
| `kv_graph::kv_set` | `MemoryGraph::kv_put` | `client.kv_set(ns, key, &value)` |
| `tool_memory::tool_rule_list` | `MemoryToolMemory::tool_rules` | `tool_memory_store(memory).list_rules(tool)` |
| `tool_memory::tool_rule_delete` | `MemoryToolMemory::delete_tool_rule` | `tool_memory_store(memory).delete_rule(tool, id)` |

Two **agent tools** followed the same route:

| Tool | Contract method | Note |
| --- | --- | --- |
| `memory_tools_list` | `MemoryToolMemory::tool_rules` | 1:1 — same rules, same order, same serialization. |
| `memory_tools_put` | `MemoryToolMemory::put_tool_rule` + `tool_rules` | The contract method returns unit while the tool answers with the *stored* rule, so the write is followed by a read-back on the id `ToolMemoryRule::new` generated before the write. Exact, not lossy: there is no server-assigned identity, and `tool_memory_namespace` normalises the caller's raw `tool_name` the same way the write did. A concurrent delete in that window errors rather than fabricating a rule. |

`memory_tools_put` therefore now refuses under the `readonly` autonomy tier
with `"memory guard: "`-prefixed text, and store-level validation errors arrive
as `MemoryError::Invalid` rather than as a raw string. Both are intended.

**Three deltas ride along, and they are the point of the milestone, not
accidents:**

1. **Tier enforcement.** A write now goes through `ToolOperation::Act`, so a
   `readonly` autonomy tier refuses it and the hourly action budget is charged
   one unit. Reads take `ToolOperation::Read`, which `SecurityPolicy` answers
   `Ok` for unconditionally today.
2. **Error strings gain a method prefix.** The driver wraps host failures
   through `host_error(context, error)`, so `"<orig>"` becomes
   `"put_document: <orig>"`. Additive context, never a swallowed cause.
3. **Taint may be raised.** `doc_put` still passes `MemoryTaint::Internal`; the
   guard's `stamp_taint` promotes it to `ExternalSync` when the turn runs under
   a source scope. It can never launder the other direction.

Redaction is a byte-identical pass-through for an embedded driver, and the
ambient source scope is applied only on `MemoryTree::query_source`, so neither
changes anything here.

## The allowlist

### A. Legitimate residents — the driver, the seam, the bind site

| Path | Reason |
| --- | --- |
| `memory/tinycortex/sync.rs` | The engine seam. |
| `memory/global.rs` | The process-global slot itself. |
| `memory/ops/helpers.rs` | Defines `active_memory_client`. |
| `memory/ops/guard.rs`, `guard_tests.rs` | The guarded resolver; matches only in prose and in its own fallback. |
| `memory/ops/provider.rs` (`.unguarded_provider(`) | Health probe on the bound driver; a liveness probe is not product code. |
| `core/cli_capability.rs` (`binding::for_workspace(`) | The CLI's capability gate (`kernel.md` §3.3's one exception to "degradation is absence"). Reads the driver id and advertised capability set only — the same two values `memory.provider_status` already returns over RPC — and never reaches memory content. No CLI subcommand except `run`/`serve` builds a `CoreContext`, so `CoreContext::memory()` resolves to nothing and there is no guard to route through. `core/memory_cli.rs` calls `bound_memory_driver_for` rather than binding itself. |
| `core/subsystems_cli.rs` | The `openhuman subsystems` slot table. Delegates to `memory_subsystem_status` (which itself resolves the binding in `memory/ops/provider.rs`, already allowlisted above), so `subsystems_cli.rs` never touches `binding::for_workspace(` directly — the CLI's command arms go through `bound_memory_driver_for`. |

### B. Unguardable raw SQLite — `profile_conn()`, out of scope for M4

No decorator can wrap an `Arc<Mutex<rusqlite::Connection>>`. These reach the
profile / facet tables beneath all seven policy steps. **This is why "the guard
is the only path" is not yet a true invariant.**

> **Update (memory module port).** The `agent/learning/*` facet bypasses are
> gone. They were justified by "the contract has no profile family"; it now has
> [`MemoryProfile`], and the learning subsystem reads and writes facets through
> the bound driver, guard included. `agent/learning/schemas.rs` and
> `agent/learning/tools.rs` no longer appear below at all, and
> `agent/learning/startup.rs` keeps two entries: a `#[cfg(test)]`-only
> construction the scanner cannot brace-track, and a boot-time
> `binding::for_workspace(` that resolves a **guard** (not a raw client) for a
> known workspace, exactly as `active_memory_guard`'s own no-ambient-context
> fallback does.

| Path | Sites |
| --- | --- |
| `vendor/tinymemory/crates/tinymemory-core/src/store/identity.rs` | 2 — the engine's cross-toolkit identity matcher resolves its global client and then reads the typed profile store. This is below the module contract, not a host-side path. |

TinyMemory v1.13.4 removed the old in-process Composio sync pipeline, so its
former profile-store entries are intentionally absent. The identity matcher is
the remaining vendor-internal reader counted by this guard.

### C. Needs a concrete engine type the contract does not expose

| Path | Reason |
| --- | --- |
| `agent/experience/ops.rs` | `AgentExperienceStore::new` takes `Arc<dyn Memory>`; the non-`"memory"` subdir branch also builds `UnifiedMemory::new_with_memory_dir` directly — a per-profile store the binding has no concept of. |
| `agent/harness/session/builder/factory.rs` | `.memory_handle()` → `Arc<dyn Memory>`. |
| `flows/tinyflows/memory_adapter.rs` | Returns `Arc<dyn Memory>` to satisfy a tinyflows engine trait. The contract has no `Arc<dyn Memory>` door. |
| `flows/bus.rs` | `resolve_memory() -> Option<Arc<dyn Memory>>`, and carries a `#[cfg(test)] memory_override` seam a guard would bypass. |

### D. No contract method exists, or the wire shape would change

| Path | Reason |
| --- | --- |
| `memory/ops/sync.rs` | `client.ingestion_state().snapshot()` — queue telemetry, absent from the contract. |
| `flows/ops.rs` | The production namespace clear uses `MemoryDocuments`; only the directly injected `MemoryClientRef` test seam remains raw. |
| `integrations/composio/schemas.rs` | Passes `&MemoryClientRef` into `user_scopes::save`. |
| `memory/sync/composio/providers/user_scopes.rs`, `types.rs` | Same `&MemoryClientRef` parameter shape. |
| `agent/learning/linkedin_enrichment.rs` | `MemoryClient::store_skill_sync` — derives the `skill-<id>` namespace and stamps every write `MemoryTaint::ExternalSync`, so the subconscious gate can see the provenance through the persistence layer. No provider family exposes either, and reimplementing them at the call site is the duplication the method exists to prevent. **It does not get the opaque-`document_id` key protection:** this caller passes `None`, and `store_skill_sync` names it as the exception — the key stays the title (`LinkedIn profile: {url}`) and does go through `upsert_document`'s secret/PII key guard. Harmless in practice for a LinkedIn URL, recorded because this column is what a future reviewer relies on to tell deliberate from forgotten, and a reason that overclaims is worse than none. **Newly listed, not newly bypassed:** this site previously reached the same client through `MemoryClient::new_local()`, for which the scanner had no needle — and which pinned `~/.openhuman` regardless of `OPENHUMAN_WORKSPACE`, so on a scoped host the profile was written to a store nothing else reads. |

### E. Tests

`flows/ops_tests.rs`, `flows/tinyflows/memory_node_e2e_tests.rs`,
`integrations/composio/ops_tests.rs`, `core/runtime/context.rs` (its `#[cfg(test)]`
module).

TinyMemory v1.13.4 removed the in-process Composio sync test support along with
the pipeline itself. The remaining vendored global-client use is the
engine-internal identity matcher described above.

## Honest scorecard

The document listing/mutation handlers, the full KV/graph handler family, the
tool-memory handlers, and flow namespace cleanup now use the shared memory API.
Raw profile/facet access and consumers whose foreign traits require
`Arc<dyn Memory>` remain unguarded and are enumerated above. The defensible
claim is therefore:

> Every memory RPC handler covered by a shared capability family routes through
> the guard, and every remaining bypass is enumerated here with a reason and
> pinned by a drift guard.

"Impossible to skip by construction" is **not** true until `memory_handle()`
is gone and the profile/facet tables have a capability family to be guarded
against.

# Phase 3 — Bounded multi-context readiness (Stage C)

**Status:** exit criterion **demonstrated for the first domain** (`people`);
remaining domains + `RPC_TOKEN`/subscriber relocation are follow-ups.
**Goal:** two `CoreContext`s in one test process serve memory/people/config
reads without cross-talk. That sentence is the exit criterion — **not** "zero
`OnceLock`s". Anything not needed for it stays process-scoped and gets
documented instead of refactored.

## Delivered

- **Isolation primitive** (Phase 2): `CoreContext::scope` / `current` +
  `DEFAULT_CONTEXT`, scoped at the dispatch chokepoint. Unit-tested
  (`scope_sets_current_context`, `nested_scope_overrides_then_restores`).
- **First per-context store**: `people::store::for_workspace(dir)` opens/caches a
  store per workspace, and `CoreContext::people()` resolves it for the context's
  workspace — additive alongside the legacy `people::store::get()` global, so the
  ~40 existing people handlers are untouched and migrate to
  `CoreContext::current()?.people()` incrementally.
- **Exit test (people)**: `people_store_is_isolated_per_context_workspace` — two
  contexts over distinct workspaces resolve isolated stores; one context always
  resolves the same cached store. This is the exit criterion, realized for the
  first migrated domain.

## Remaining (follow-ups)

- Repeat the `for_workspace` + `CoreContext::<domain>()` pattern for `memory`,
  `config`, `attachments`, then migrate their handlers to read through
  `CoreContext::current()`.
- Move `RPC_TOKEN` onto the context and make event-bus subscriber registration
  per-context (below) — only needed once a host actually runs >1 context in a
  process (the fleet uses process-per-user, so this is not on its path).

## Scope

### 3.1 Per-context state

| Item                         | Today                                                                                                                                 | After                                                                                                                                                                  |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| RPC bearer                   | `RPC_TOKEN: OnceLock<String>` (`src/core/auth.rs:75`)                                                                                 | field on `CoreContext` — it gates a per-runtime HTTP listener; `auth::get_rpc_token` facade reads the default context                                                  |
| Workspace / active user      | resolved once from `OPENHUMAN_WORKSPACE` → `active_user.toml` → `"local"` (`config/schema/load/dirs.rs:299`, `load_user_state.rs:21`) | resolution runs in `CoreBuilder::build` and the result is a `CoreContext` field; the marker-file chain remains the _default_ when the embedder passes no workspace     |
| Event-bus domain subscribers | `register_domain_subscribers` under `std::sync::Once` (`src/core/jsonrpc.rs:2232`)                                                    | registration keyed per context (subscription handles owned by `CoreContext`, dropped on shutdown); process-level dedupe kept only for genuinely process-global targets |
| Stores migrated in phase 2.c | global facade → default context                                                                                                       | second context constructs its own `StorageBackend::WorkspaceFs` instance over its own workspace dir                                                                    |

### 3.2 Permanently process-scoped (documented, not refactored)

| Item                                                                                        | Why it stays global                                                                                                                                               |
| ------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Keyring / master key (`keyring::init_master_key`, `src/lib.rs`)                             | OS keychain is per-process/user by nature                                                                                                                         |
| Sentry (`src/main.rs`)                                                                      | binary concern, not library concern — embedders bring their own                                                                                                   |
| `NativeRegistry` (`event_bus/native_request.rs:329`)                                        | internal typed dispatch; handlers are stateless routers to context-owned state                                                                                    |
| Env vars (`OPENHUMAN_CORE_RPC_URL` set_var, `jsonrpc.rs:2010`; `OPENHUMAN_WORKSPACE` reads) | child-process contract; this is exactly why multi-_tenant_ hosting is process-per-user (phase 4), and it is documented as a single-runtime-per-process constraint |

This table ships in the README of this plan (or the drift ledger) as the
authoritative "what is process-scoped and why" inventory for embedders.

## Exit test

`tests/` integration test: construct two `CoreContext`s over two temp
workspaces in one process (`ServiceSet::none()`), write a person + a config
value + a memory item through context A, assert context B sees none of them
and vice versa; both contexts shut down cleanly (subscription handles
dropped, no `Once` poisoning).

Desktop path must be bit-identical: the single-context Tauri/CLI flow never
constructs a second context; the test-only second context is the only
consumer until in-process multi-workspace is deliberately productized.

## Risks & mitigations

| Risk                                                                         | Mitigation                                                                                                           |
| ---------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| Double-firing subscribers when two contexts register                         | subscription handles owned per context; bus events carry context/workspace identity where a handler writes to stores |
| Hidden global discovered late (some `*::global()` not in the phase-2 ledger) | the exit test is the detector; fix-forward per store, ledger updated                                                 |
| `Once` removal regresses single-context boot                                 | keep `Once` semantics for the default context; per-context path only activates for non-default contexts              |

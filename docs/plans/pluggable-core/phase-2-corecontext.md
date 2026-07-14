# Phase 2 — `CoreContext` ownership, ambient context, registry collapse, store traits

**Status:** Stage A + ambient-context + registry-collapse **DONE**; per-domain
store-trait migration remains.
**Goal:** make state reachable _through_ a context instead of _only_ through
process globals, without a big-bang DI rewrite.

## 2.a Handler context access — ambient scope, not a signature sweep (Stage B)

**Implemented (deviation from the original plan, with rationale below):** the
literal signature change to `ControllerHandler` was rejected after measuring its
true cost. `ControllerHandler` fn pointers appear at **846 handler definitions
across 109 files, plus ~381 direct call sites** — a change of that type must
land atomically to compile, needs a default-context `OnceLock` with fallbacks
for the many callers that never build a runtime (CLI one-shots, tests, MCP
dispatch), and yields **zero** functional value until each domain later reads
`ctx` (Stage C). That is disproportionate churn and a high risk of a
non-compiling tree.

Instead, the **goal** — every handler can reach the `CoreContext` for the
current dispatch — is delivered with a `tokio::task_local!` ambient context:

- `CoreContext::init` registers the first-built context as the process
  `DEFAULT_CONTEXT` (`OnceLock<Arc<CoreContext>>`).
- The dispatch chokepoint `all::try_invoke_registered_rpc` wraps each handler
  future in `CoreContext::scope(ctx, fut)`, where `ctx` is
  `CoreContext::current()` — the active scope if any (so nested dispatches stay
  in the same tenant context), else the default.
- Handlers reach it via `CoreContext::current() -> Option<Arc<CoreContext>>`.
  Controller handlers stay **bare `fn` pointers** — zero per-handler churn.
- A domain migrates off a process global by reading its store handle from
  `CoreContext::current()` instead. Once its state lives on the context, two
  contexts dispatched under distinct `CoreContext::scope`s read isolated state —
  exactly the Phase 3 exit criterion, verified by the unit tests in
  `src/core/runtime/context.rs` (`scope_sets_current_context`,
  `nested_scope_overrides_then_restores`).

One implementation note: the extra future layer at the chokepoint pushed the
`Send` auto-trait solver past the default depth on the deepest axum→tinyagents
routes; the scoped future is re-boxed into a `ControllerFuture` and the crate
sets `#![recursion_limit = "256"]` (both in `src/lib.rs` / `src/core/all.rs`).

This is strictly a better realization of Stage B's intent; the explicit-param
approach is not planned.

## 2.b Registry collapse

Replace the three hand-maintained parallel lists in `src/core/all.rs`
(handlers `:105-344`, schemas `:375-509`, namespace descriptions `:530-690`)
with one per-domain struct:

```rust
pub struct DomainRegistration {
    pub namespace: &'static str,
    pub description: &'static str,
    pub controllers: Vec<RegisteredController>,   // schema + handler already paired
    pub cli: Option<CliHandler>,                  // absorbs CLI_ADAPTERS (all.rs:88)
}
fn all_domains() -> Vec<DomainRegistration> { vec![about_app::domain(), /* … one line per domain … */] }
```

**Implemented (partial — the drift-elimination half):**

- `all_controller_schemas()` now **derives** the schema list from the
  registered controllers (`registry().iter().map(|c| c.schema.clone())`), so the
  parallel `build_declared_controller_schemas()` list is deleted and
  `validate_registry`'s declared-vs-registered cross-check is removed —
  the two lists can no longer drift. `validate_registry` keeps the
  duplicate-method / empty-namespace / duplicate-required-input checks on the
  registered set. Obsolete drift unit tests were removed.
- **Explicitly rejected:** `inventory`/linkme link-section auto-registration.

**Deferred (cosmetic, no correctness value):** folding the per-domain
`all_X_controller_schemas()` fns and the `namespace_description` match into a
single `DomainRegistration` struct. The per-domain schema fns still exist but
are no longer aggregated centrally; collapsing them further is churn across 109
domains for no behavior change, tracked as a follow-up.

## 2.c Per-domain migration + store traits

Domains migrate `_ctx` → real context usage opportunistically; **required**
(because multi-context isolation in phase 3 needs them) for: `memory`
(tinycortex seam handle), `people`, `attachments`, `config`. Everything else
migrates when touched.

Storage abstraction lands here, at the handle boundary:

```rust
impl CoreContext {
    pub fn people(&self) -> Arc<dyn PeopleStore>;
    pub fn attachments(&self) -> Arc<dyn AttachmentStore>;
    // … per-domain accessors added as domains migrate
}

pub enum StorageBackend { WorkspaceFs }   // CoreBuilder::storage(..); only impl for now
```

- Traits are carved **per domain as it migrates** (drift-ledger column
  "store trait extracted?"), never as one up-front storage rewrite.
- `WorkspaceFs` wraps the existing SQLite/JSON-on-disk stores byte-for-byte;
  the global facades (`*::global()`) delegate to the default context's
  handles so both paths hit the same instance.
- Host-owned stores only (people, attachments, cost/x402 ledgers, config
  state, run ledgers). `tinyagents` (`StoreRegistry`, checkpointers) and
  `tinycortex` already own their internal store interfaces — the context
  holds _handles to_ those seams, it does not re-abstract them.
- Adding a remote backend (e.g. Postgres) must become "new `StorageBackend`
  impl", but shipping one is out of scope.

## Risks & mitigations

| Risk                                     | Mitigation                                                                             |
| ---------------------------------------- | -------------------------------------------------------------------------------------- |
| Sweep-size diff unreviewable             | three PR series: signature-only → registry collapse → per-domain                       |
| Global facade and context handle diverge | facade delegates to default context (single instance); parity asserts in debug builds  |
| Store trait too narrow, churns later     | extract traits from _observed_ handler usage per domain, not speculatively             |
| Coverage gate on mechanical diffs        | signature/registry PRs carry existing tests; per-domain PRs add store-trait unit tests |

## 2.d `DomainSet` — the runtime composition axis (#4796)

`DomainSet` (in `src/core/runtime/builder.rs`, sibling of `ServiceSet`) is the
**runtime** axis that selects which domain *families* exist, complementing
`ServiceSet` (which selects background services). One flag per `DomainGroup`
(`src/core/all.rs`); presets `full()` (default — byte-identical to pre-#4796),
`harness()` (agent + memory + threads + config + security only), `none()`.

Mechanism (Shape B — filter seam, **not** the full per-domain
`DomainRegistration` struct collapse, which remains phase-2-deferred):

- Every controller is tagged with its `DomainGroup` once, at the single
  registration site (`build_registered_controllers` /
  `build_internal_only_controllers`), via a `GroupedController { group,
  controller }` wrapper — the ~109 domain modules keep returning bare
  `RegisteredController` lists and never learn about groups.
- The live surface filters by the **ambient** `CoreContext::domains()`:
  `all_registered_controllers` / `all_controller_schemas` (so `/schema` omits
  gated namespaces), `try_invoke_registered_rpc` (gated method ⇒ `None`, i.e.
  unknown-method), the agent tool surface (`tools::ops::all_tools_with_runtime`
  post-filter by `tool_group`), workspace-bound store init (`init_stores`), and
  domain event subscribers (`register_domain_subscribers`).
- No active context (pre-boot unit tests) ⇒ no filtering (treated as full).
- Per-gate Cargo `[features]` (children #4797–#4804) narrow the *compile-time*
  surface further; `DomainSet` is the runtime axis they compose with.

## Verification

- `pnpm test:rust` + `json_rpc_e2e` green after each sub-series.
- Drift ledger lists every domain with columns: signature migrated / context
  usage / store trait / notes.
- `all::try_invoke_registered_rpc` installs an ambient `CoreContext::scope`
  around registered handler futures, and tests verify `CoreContext::current()`
  propagation through registered RPC invocation.
- `DomainSet`: `full_registration_is_byte_identical`,
  `harness_excludes_gated_namespaces`, `dispatch_returns_none_for_gated_method`,
  `group_mapping_smoke` (`src/core/all_tests.rs`) +
  `domain_set_presets_have_expected_flags` (`builder.rs`).

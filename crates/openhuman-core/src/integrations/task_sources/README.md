# task_sources

Proactive ingestion of work items from external tools. A **task source** is a user-configured pull from a Composio-backed provider (GitHub, Notion, Linear, ClickUp) with a per-provider filter. A periodic poll runs a fetch → dedup → enrich → route pipeline that records each item in the ingestion ledger and, for proactive sources, dispatches a triage turn so an agent can start working immediately. **The fetch stage is currently a stub**: `ComposioProvider::fetch_tasks` was deleted upstream (tinymemory v1.13.4) with no replacement, so `pipeline::fetch_tasks_unavailable` refuses every toolkit and only the surrounding stages (dedup, enrichment, routing, storage, reconciliation) are live, see [Notes](#notes--gotchas). The domain mirrors the `cron` layering: [`mod.rs`](./mod.rs) is export-only, business logic lives in sibling modules, persistence is SQLite, and the RPC surface is wired through [`schemas.rs`](./schemas.rs).

## Responsibilities

- Persist per-source configs (provider + filter + schedule + routing target + optional pinned connection / static executor).
- Periodically poll enabled sources ([`periodic.rs`](./periodic.rs)) on a global 10-minute tick honoring per-source `interval_secs` (floored to 60s).
- Translate a typed `FilterSpec` into the provider-agnostic `TaskFetchFilter` (`FilterSpec::to_fetch_filter`, owned by `tinyconnectors_bus::vocabulary`; `ProviderSlug`, `SourceTarget` and `FetchReason` live there too); the fetch itself is stubbed (`pipeline::fetch_tasks_unavailable`) until a task-fetch surface exists again.
- Dedup ingested items with an edit-aware SHA-256 content hash; re-ingest only when the upstream task changed ([`store.rs`](./store.rs) + [`pipeline.rs`](./pipeline.rs)).
- Deterministically enrich raw tasks into agent-ready ones, urgency heuristic, summary, linked assignee, templated agent prompt ([`enrich.rs`](./enrich.rs)).
- Route enriched tasks: for proactive sources, dispatch a triage turn through the same path Composio webhooks use; collect-only sources stop at the ledger ([`route.rs`](./route.rs)).
- Fire a one-shot fetch when a matching Composio connection is created ([`bus.rs`](./bus.rs)).
- Expose an `openhuman.task_sources_*` RPC surface for CRUD, manual fetch/sync, filter preview, container listing, ingested-task listing, and status (`schemas.rs` + [`ops.rs`](./ops.rs)).

## Key files

| File | Role |
| --- | --- |
| `crates/openhuman-core/src/integrations/task_sources/mod.rs` | Export-only: module docstring, `mod`/`pub mod` decls, `pub use` re-exports, and the `all_task_sources_*` controller registry pair. |
| [`crates/openhuman-core/src/integrations/task_sources/types.rs`](./types.rs) | Re-exports the connector vocabulary types (`ProviderSlug`, `FilterSpec`, `SourceTarget`, `FetchReason`, owned by `tinyconnectors_bus::vocabulary`) and defines the host-owned task-source types (`TaskSource`, `TaskSourcePatch`, `EnrichedTask`, `FetchOutcome`) plus `provider_can_fetch`. |
| `crates/openhuman-core/src/integrations/task_sources/store.rs` | SQLite persistence (`<workspace>/task_sources/sources.db`): `task_sources` + `ingested_tasks` tables, dedup `content_hash`, migrate-on-open. |
| `crates/openhuman-core/src/integrations/task_sources/ops.rs` | RPC-facing business logic returning `Outcome<T>`: `list`/`get`/`add`/`update`/`remove`/`fetch`/`sync`/`list_tasks`/`preview_filter`/`list_databases`/`status`. |
| `crates/openhuman-core/src/integrations/task_sources/schemas.rs` | `task_sources` controller schemas + `all_controller_schemas` / `all_registered_controllers` + thin `handle_*` param parsers delegating to `ops.rs`. |
| `crates/openhuman-core/src/integrations/task_sources/pipeline.rs` | `run_source_once`: the infallible fetch → dedup → enrich → route pass shared by poll, manual RPC, and connection hook; publishes domain events. Holds the `fetch_tasks_unavailable` stub and its rationale doc comment. |
| `crates/openhuman-core/src/integrations/task_sources/enrich.rs` | Deterministic, dependency-free `enrich_task`: urgency heuristic, summary, linked assignee, agent prompt. No LLM call. |
| `crates/openhuman-core/src/integrations/task_sources/route.rs` | `route_enriched`: dispatches a scheduler-gated triage turn for proactive sources; collect-only sources are a no-op past the ledger. |
| `crates/openhuman-core/src/integrations/task_sources/periodic.rs` | `start_periodic_poll`: global tick scheduler; per-source due-timing in a process-global map; `run_one_tick` is `pub(crate)` for tests. |
| `crates/openhuman-core/src/integrations/task_sources/bus.rs` | `TaskSourcesConnectionSubscriber` + `register_task_sources_subscriber`: one-shot fetch on `ComposioConnectionCreated`. |
| [`crates/openhuman-core/src/integrations/task_sources/tools.rs`](./tools.rs) | LLM-callable wrappers over `ops.rs`: see [Agent tools](#agent-tools) below. |
| [`crates/openhuman-core/src/integrations/task_sources/store_tests.rs`](./store_tests.rs) | Sibling test suite for `store.rs`. |
| [`crates/openhuman-core/src/integrations/task_sources/pipeline_tests.rs`](./pipeline_tests.rs) | Sibling test suite for `pipeline.rs`. |
| [`crates/openhuman-core/src/integrations/task_sources/tools_tests.rs`](./tools_tests.rs) | Sibling test suite for `tools.rs`. |

## Public surface

Re-exported from `mod.rs`:

- Types: `TaskSource`, `TaskSourcePatch`, `EnrichedTask`, `FetchOutcome`; `FilterSpec`, `ProviderSlug`, `SourceTarget` and `FetchReason` are re-exported from `tinyconnectors_bus::vocabulary` through `types.rs`; plus `NormalizedTask`, `TaskContainer`, `TaskFetchFilter`, `TaskKind` re-exported from `crate::integrations::composio::contract` (`pub use crate::integrations::composio::contract::{NormalizedTask, TaskContainer, TaskFetchFilter, TaskKind};` in `mod.rs`).
- Functions: `start_periodic_poll`, `run_source_once`.
- Constant: `TASK_SOURCES_THREAD_ID` (`"task-sources"`).
- RPC registry: `all_task_sources_controller_schemas`, `all_task_sources_registered_controllers`, `task_sources_schemas`.

## RPC / controllers

Namespace `task_sources` (methods `openhuman.task_sources_<function>`):

| Function | Description |
| --- | --- |
| `list` | List all configured sources. |
| `get` | Fetch one source by id. |
| `add` | Create a source; missing schedule/target/cap fall back to `[task_sources]` config defaults. |
| `update` | Apply a partial `TaskSourcePatch`. |
| `remove` | Delete a source by id (cascades `ingested_tasks`). |
| `fetch` | Fetch one source now (`FetchReason::Manual`) and route new tasks. |
| `sync` | Run `fetch` for every enabled source; returns one `FetchOutcome` per source. |
| `list_tasks` | List recently ingested tasks for a source (newest first, default limit 50). |
| `preview_filter` | Dry-run a filter, fetch matching tasks WITHOUT routing/recording. Currently always errors (fetch stub). |
| `list_databases` | List selectable containers (Notion databases) for a provider. Currently always errors (fetch stub). |
| `status` | Domain master switch + default interval + source counts. |

Handlers parse params and delegate to `ops.rs`; schemas reference `FilterSpec`, `TaskSource`, `TaskSourcePatch`, `FetchOutcome`, `NormalizedTask`. Registered into the global registry via [`crates/openhuman-core/src/core/all.rs`](../../core/all.rs).

## Agent tools

`tools.rs` wraps `ops.rs` as thin LLM-callable shims, each parses args,
calls the matching `ops` function, and emits its `Outcome::value` as JSON.
Read/observe tools are default-enabled; the persistent-config mutators are
default-OFF and must be explicitly allowlisted via [`tools/user_filter.rs`](../../tools/user_filter.rs)
(the `task_source_manage` filter group covers `add`/`update`/`remove`).

| Tool name | Struct | Backing op | Default |
| --- | --- | --- | --- |
| `task_source_list` | `TaskSourceListTool` | `ops::list` | enabled |
| `task_source_get` | `TaskSourceGetTool` | `ops::get` | enabled |
| `task_source_fetch` | `TaskSourceFetchTool` | `ops::fetch` | enabled |
| `task_source_list_tasks` | `TaskSourceListTasksTool` | `ops::list_tasks` | enabled |
| `task_source_preview_filter` | `TaskSourcePreviewFilterTool` | `ops::preview_filter` | enabled |
| `task_source_status` | `TaskSourceStatusTool` | `ops::status` | enabled |
| `task_source_add` | `TaskSourceAddTool` | `ops::add` | OFF (`task_source_manage`) |
| `task_source_update` | `TaskSourceUpdateTool` | `ops::update` | OFF (`task_source_manage`) |
| `task_source_remove` | `TaskSourceRemoveTool` | `ops::remove` | OFF (`task_source_manage`) |

Re-exported into the global agent tool registry via
[`crates/openhuman-core/src/tools/mod.rs`](../../tools/mod.rs)
(`pub use crate::integrations::task_sources::tools::*;`).

`task_source_fetch` and `task_source_preview_filter` advertise themselves
as **UNAVAILABLE** in their tool descriptions for the same reason the fetch
stage is stubbed; calling either always fails until a task-fetch surface is
restored.

## Events

Publishes (via `BUS.publish`, domain `"task_sources"` in [`core/events.rs`](../../core/events.rs)):

- `DomainEvent::TaskSourceFetched`: after a successful fetch pass (counts: fetched/routed/skipped).
- `DomainEvent::TaskSourceTaskIngested`: per newly routed task (provider, external_id, title, urgency).
- `DomainEvent::TaskSourceFetchFailed`: on a failed pass (error string).

Subscribes:

- `DomainEvent::ComposioConnectionCreated` (domain filter `["composio"]`) via `TaskSourcesConnectionSubscriber`: fires a one-shot `ConnectionCreated` fetch for matching enabled sources. Registered once at startup (`register_task_sources_subscriber`, idempotent `OnceLock` handle).

Startup wiring is split across three sites; both entry points are idempotent
(`OnceLock`), so the overlap is harmless:

- [`crates/openhuman-core/src/core/runtime/subscribers.rs`](../../core/runtime/subscribers.rs) calls
  `crate::integrations::task_sources::bus::register_task_sources_subscriber()`.
- [`crates/openhuman-core/src/core/runtime/services.rs`](../../core/runtime/services.rs) calls
  `crate::integrations::task_sources::start_periodic_poll()` when the
  `ServiceSet`'s task-source polling bootstrap job is enabled.
- [`crates/openhuman-core/src/channels/runtime/startup/start_channels.rs`](../../channels/runtime/startup/start_channels.rs)
  (`start_channels_inner`) calls both `register_task_sources_subscriber()`
  and `start_periodic_poll()` again for the channels runtime path.

## Persistence

SQLite at `<workspace_dir>/task_sources/sources.db` (WAL, 5s busy timeout, migrate-on-open):

- **`task_sources`**: configured sources, provider, optional connection_id/name, enabled, filter JSON, interval_secs, target, max_tasks_per_fetch, created_at, and last_fetch_at/last_status.
- **`ingested_tasks`**: per-(source, external_id) dedup ledger, edit-aware `content_hash` (SHA-256 over title/body/status/updated_at/url), normalized task `payload`, `ingested_at`. The `card_id` column is a leftover from when tasks were mirrored onto a todo board; it is written as `NULL` and kept only so older databases open unchanged. FK to `task_sources` with `ON DELETE CASCADE`.

The additive idempotent `ingested_tasks.card_id` migration preserves older databases. App-level defaults (enabled flag, default interval, per-fetch cap, auto_proactive) live in config (`TaskSourcesConfig`), not the store.

### On a storage backend

When the host configured a storage backend (`OPENHUMAN_STORAGE_URL` /
`[storage] url`, see `crate::storage`), every `store` function uses
`store_documents.rs` instead of `sources.db`: the same operations on the
`tinystoragedrivers` document port, under the current call's storage scope
(the acting agent; `local` on a single-user host; refused in SaaS mode with
no acting agent). Collections `task_sources` (one per source) and `ingested_tasks` (one
per `(source_id, external_id)`). `update_source` applies the patch under
compare-and-swap, and removing a source removes its ledger entries (the
SQL cascade). With no backend configured (the desktop default)
`sources.db` is used as described above.

## Dependencies

- `crate::core::all`: `ControllerFuture`, `RegisteredController` for the RPC registry.
- `crate::core::bus` (`BUS`) + `crate::core::events::DomainEvent`: `BUS.publish`/`BUS.subscribe` for event publish/subscribe.
- `crate::config` (+ `config::rpc`), `Config`, `load_config_with_timeout`; reads the `[task_sources]` block for defaults and the master switch.
- `crate::integrations::composio::contract`: `NormalizedTask`, `TaskContainer`, `TaskFetchFilter`, `TaskKind` (contract types re-exported from `tinymemory_api::composio::tasks`). The old `get_provider` / `ProviderContext` / `ComposioProvider::fetch_tasks` registry no longer exists; `mod.rs`'s intra-doc link to `fetch_tasks` is stale.
- `crate::triage`: `run_triage`, `apply_decision`, `TriageOutcome`, `TriggerEnvelope`; dispatches the proactive agent turn for `AgentTodoProactive` sources.
- `crate::platform::llm_gate`: `wait_for_capacity` capacity semaphore; gates proactive triage turns behind background-AI throttling.

## Used by

- `crates/openhuman-core/src/core/all.rs`: registers controllers + schemas into the global RPC registry.
- `crates/openhuman-core/src/core/runtime/subscribers.rs`: at startup registers the connection subscriber (bus.rs).
- `crates/openhuman-core/src/core/runtime/services.rs`: at startup starts the periodic poll as part of the task-source polling bootstrap job.
- `crates/openhuman-core/src/channels/runtime/startup/start_channels.rs`: `start_channels` registers the subscriber and starts the poll for the channels runtime.
- `crates/openhuman-core/src/core/events.rs`: defines/classifies the three `TaskSource*` event variants under domain `"task_sources"`.
- `crates/openhuman-core/src/tools/mod.rs`: re-exports `tools.rs`'s agent tools into the global tool registry.
- [`crates/openhuman-core/src/config/schema/`](../../config/schema/): `TaskSourcesConfig` block feeding domain defaults.

## Notes / gotchas

- **Fetch is stubbed (tinymemory v1.13.4).** `pipeline::fetch_tasks_unavailable` and `ops::preview_filter` / `ops::list_databases` return an error for every toolkit: the engine's in-process Composio pipeline was deleted upstream and the `tinyconnectors` module exposes only `Execute` and `Sync`, no structured task-fetch member. Re-deriving each toolkit's action selection and response parsing over `Execute` is per-provider work, so the domain refuses cleanly rather than half-porting. Periodic passes therefore record a failed fetch and publish `TaskSourceFetchFailed`.
- **Periodic cadence is coarse.** `TICK_SECONDS = 600` is the effective lower bound: any `interval_secs` shorter than 10 minutes is rounded up to the tick. A misconfigured `interval_secs = 0` is floored to `MIN_INTERVAL_SECONDS = 60`. The first immediate-fire tick is skipped so startup isn't slammed.
- **Pipeline is infallible at the boundary.** `run_source_once` captures any error into `FetchOutcome::error` (and a failure event) so the scheduler loop never unwinds.
- **Route-then-mark ordering.** A task is marked ingested only after routing succeeds, so a routing failure retries next pass instead of being silently dropped.
- **Edit-aware dedup.** `content_hash` includes `url` deliberately (it drives external write-back); a changed hash re-ingests and re-routes.
- **`update_source` TOCTOU.** Documented theoretical read-modify-write window across three connections; acceptable at settings-panel scale.
- **Enrichment is intentionally LLM-free**: deterministic and unit-testable; the heavy reasoning happens in the downstream triage turn.
- `clear_all` exists for the E2E `test_reset` RPC.

## Further reading

- [Parent module README](../README.md)
- [Triggers](../../../../../docs/gitbooks/en/features/integrations/triggers.md)
- [Goals and todos](../../../../../docs/gitbooks/en/features/goals-and-todos.md)

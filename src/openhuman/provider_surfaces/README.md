# provider_surfaces

Local assistive surfaces for third-party provider apps. This domain owns a normalized provider-event model and an in-memory **respond queue** that sits above embedded webviews (and future API-first integrations), so the assistant can surface actionable items (messages, mentions, etc.) from providers like LinkedIn/Gmail in a single local-first list. The current cut is an intentionally minimal scaffold: it exposes two RPC controllers, keeps state in process memory, and is wired into the controller registry ahead of behavioral/SQLite work.

## Responsibilities

- Define a normalized inbound `ProviderEvent` contract (provider slug, account id, event kind, entity id, optional thread/title/snippet/sender/deep-link, timestamp, `requires_attention`, optional raw payload).
- Ingest provider events and upsert them into a local respond queue keyed by `provider:account_id:event_kind:entity_id`.
- List the respond queue (newest-first) for assistive UI surfaces.
- Bound queue growth with a soft cap (500 items, oldest-from-tail dropped) under provider firehose volume.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/provider_surfaces/mod.rs` | Export-only: declares submodules; re-exports `all_provider_surfaces_controller_schemas` / `all_provider_surfaces_registered_controllers`. |
| `src/openhuman/provider_surfaces/types.rs` | Serde domain types: `ProviderEvent`, `RespondQueueItem`, `RespondQueueListResponse`. Snake_case contract shared by request and response. |
| `src/openhuman/provider_surfaces/ops.rs` | Business logic / entry points: `ingest_event`, `list_queue`. Wrap results in `ApiEnvelope` + `RpcOutcome`. Contains the inline test suite. |
| `src/openhuman/provider_surfaces/store.rs` | In-memory persistence: process-global `RESPOND_QUEUE` (`OnceLock<Mutex<Vec<…>>>`), `upsert_queue_item`, `list_queue_items`, `clear_queue` (test-only). |
| `src/openhuman/provider_surfaces/schemas.rs` | Controller registry: `ControllerSchema`s + `handle_*` fns delegating to `ops.rs`. Inline schema tests. |
| `src/openhuman/provider_surfaces/rpc.rs` | Docstring-only placeholder; no code. The handler delegation lives in `schemas.rs`, not here. |

## Public surface

- `types::ProviderEvent` — inbound normalized provider event (`#[serde(deny_unknown_fields)]`).
- `types::RespondQueueItem` — queue entry (adds `id` and `status`, default `"pending"`).
- `types::RespondQueueListResponse` — `{ items, count }`.
- `ops::ingest_event(ProviderEvent)` / `ops::list_queue(EmptyRequest)` — async handlers returning `RpcOutcome<ApiEnvelope<T>>`.
- `store::{upsert_queue_item, list_queue_items}` — used directly by `desktop_companion` (see Used by).
- Re-exported from `mod.rs`: `all_provider_surfaces_controller_schemas`, `all_provider_surfaces_registered_controllers`.

## RPC / controllers

Namespace `provider_surfaces` (two controllers, registered via `src/core/all.rs`):

| Method | Inputs | Output |
| --- | --- | --- |
| `provider_surfaces.ingest_event` | `provider`, `account_id`, `event_kind`, `entity_id`, `timestamp` (required); `thread_id`, `title`, `snippet`, `sender_name`, `sender_handle`, `deep_link` (optional); `requires_attention` (bool, defaults false), `raw_payload` (optional JSON) | Envelope containing the upserted `RespondQueueItem`. |
| `provider_surfaces.list_queue` | none | Envelope containing `{ items, count }`. |

`requires_attention` is `required: false` in the schema to match `ProviderEvent`'s `#[serde(default)]` so registry `validate_params` agrees with the struct.

## Agent tools

None — no `tools.rs`; this domain owns no agent tools.

## Events

None — no `bus.rs`; no `DomainEvent`s published or subscribed.

## Persistence

In-memory only. State lives in a process-global `RESPOND_QUEUE` (`static OnceLock<Mutex<Vec<RespondQueueItem>>>`) in `store.rs`, prepend-ordered (newest-first), soft-capped at `MAX_QUEUE_ITEMS = 500` (oldest dropped from the tail). Upsert dedupes by composite id `provider:account_id:event_kind:entity_id`. Module docstrings flag SQLite-backed persistence for normalized events, queue state, and local drafts as follow-up work — not yet present.

## Dependencies

- `crate::openhuman::memory` — `ApiEnvelope`, `ApiMeta`, `EmptyRequest` (response envelope shape + empty-request type).
- `crate::rpc::RpcOutcome` — RPC return contract.
- `crate::core::all` — `RegisteredController`, `ControllerFuture` (controller registry wiring).
- `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — controller schema types.
- External crates: `serde` / `serde_json`, `uuid` (request ids), `tracing` (debug logging with `[provider-surfaces]` prefix).

## Used by

- `src/core/all.rs` — registers the controllers/schemas into the global registry (`all_provider_surfaces_registered_controllers`, `all_provider_surfaces_controller_schemas`, and a `"provider_surfaces"` dispatch arm).
- `src/openhuman/desktop_companion/handoff.rs` — reads `store::list_queue_items()` and matches `RespondQueueItem`s to correlate desktop companion handoff actions against the queue (light-touch, read-only against the store).
- `src/openhuman/task_sources/pipeline_tests.rs` — references the queue in tests.

## Notes / gotchas

- **Scaffold, not finished domain.** Docstrings across `mod.rs`/`ops.rs`/`store.rs` explicitly call this an initial cut: state is in-memory, SQLite store + drafts + provider-specific assistive actions are deferred.
- **`rpc.rs` is empty (docstring only).** Despite the canonical module shape suggesting `rpc.rs` holds the pure-domain API, here handlers live in `schemas.rs` delegating to `ops.rs`; `rpc.rs` is a placeholder.
- **Queue id is deterministic** (`provider:account_id:event_kind:entity_id`), so re-ingesting the same entity upserts (removes + re-prepends) rather than duplicating.
- **Process-global mutable state** means tests must serialize around `RESPOND_QUEUE`; `ops.rs` tests use a `TEST_MUTEX` + `store::clear_queue()` to avoid interleaving under cargo's parallel runner. Mutex poisoning is recovered via `into_inner()`.
- **Snake_case contract is intentional and shared** between request (`ProviderEvent`) and response (`RespondQueueItem`) so callers see one consistent shape.

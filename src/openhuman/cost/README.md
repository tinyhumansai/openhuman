# cost

API-usage cost tracking and budget enforcement for the agent. Records per-call token usage and computed USD cost to an append-only JSONL file, maintains in-memory daily/monthly aggregates, exposes a budget gate (`check_budget`) and a 7-day dashboard surface over JSON-RPC. A process-global singleton tracker is shared by the agent turn loop (telemetry) and the dashboard RPC handlers so each provider call is persisted exactly once.

## Responsibilities

- Compute per-call cost in USD from token counts and per-million prices (`TokenUsage::new`), clamping non-finite/negative prices to `0.0`.
- Persist each usage event as a `CostRecord` line in `costs.jsonl` (durable: write + `sync_all`).
- Maintain cached current-day / current-month spend aggregates, rebuilt on day/month rollover.
- Enforce daily and monthly budget limits with warn-threshold signalling (`check_budget` → `BudgetCheck::{Allowed, Warning, Exceeded}`) — only when `cost.enabled`.
- Capture dashboard telemetry **unconditionally** (independent of `cost.enabled`) via `record_usage_unconditional`, so history exists before a user opts into enforcement.
- Aggregate a 7-day daily history (zero-filling gap days), monthly pace projection, budget utilisation/status, and per-model breakdown for the dashboard.
- Serve the dashboard / daily-history / summary over JSON-RPC, with a cached read-only fallback tracker when the global is uninitialised.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/cost/mod.rs` | Export-focused module root; re-exports tracker, types, global helpers, and the `all_cost_*` controller schema/registry pair. |
| `src/openhuman/cost/types.rs` | Serde domain types: `TokenUsage`, `CostRecord`, `UsagePeriod`, `BudgetCheck`, `CostSummary`, `ModelStats`, `DailyCostEntry`, `BudgetStatus`, `CostDashboard`. Cost-calc logic lives in `TokenUsage::new`. |
| `src/openhuman/cost/tracker.rs` | `CostTracker` (budget checks, recording, summaries, daily history, dashboard build) plus the private `CostStorage` JSONL persistence + aggregate-cache layer. Functions as both `ops` and `store`. |
| `src/openhuman/cost/global.rs` | Process-global `OnceCell<Arc<CostTracker>>` singleton: `init_global`, `try_global`, `record_provider_usage`, and `build_token_usage` (provider `UsageInfo` → `TokenUsage`). |
| `src/openhuman/cost/rpc.rs` | RPC-facing handlers (`dashboard`, `daily_history`, `summary`) returning `RpcOutcome<Value>`; DTO types; `resolve_tracker` with a cached fallback tracker + error-replay TTL. |
| `src/openhuman/cost/schemas.rs` | Controller schemas + `handle_*` JSON-RPC dispatchers; `all_controller_schemas` / `all_registered_controllers`. |
| `src/openhuman/cost/tracker_tests.rs` | Sibling test suite for `tracker.rs` (`#[path]`-included). |

## Public surface

From `mod.rs` re-exports:

- `CostTracker` — the tracker (`tracker`).
- `init_global`, `try_global`, `record_provider_usage` (`global`).
- `all_cost_controller_schemas`, `all_cost_registered_controllers` (`schemas`).
- Types: `BudgetCheck`, `BudgetStatus`, `CostDashboard`, `CostRecord`, `CostSummary`, `DailyCostEntry`, `ModelStats`, `TokenUsage`, `UsagePeriod`.

Notable `CostTracker` methods: `new`, `session_id`, `check_budget`, `record_usage`, `record_usage_unconditional`, `get_summary`, `get_daily_cost`, `get_monthly_cost`, `get_daily_history`, `get_dashboard`.

## RPC / controllers

Namespace `cost` (methods `openhuman.cost_*` via the registry):

| Method | Inputs | Output |
| --- | --- | --- |
| `cost_get_dashboard` | none | 7-day dashboard payload: per-day buckets, summary metrics, budget utilisation/status, per-model breakdown. |
| `cost_get_daily_history` | `days?` (u32, default 7, clamped `[1, 366]`) | Ordered daily entries, oldest first, gaps zero-filled. |
| `cost_get_summary` | none | Live session / daily / monthly cost summary. |

Handlers load config via `config_rpc::load_config_with_timeout`, then delegate to `rpc.rs`. RPC DTOs (`CostDashboardDto`, `DailyCostEntryDto`, `ModelStatsDto`, `CostSummaryDto`) add presentation fields not on the domain types — `provider` (derived from the `provider/model` prefix), `percent_of_total`, and dashboard threshold/`enabled` flags from `cost.dashboard`.

## Events

None. The module has no `bus.rs` and no `DomainEvent` publishers/subscribers.

## Persistence

- Append-only JSONL at `<workspace>/state/costs.jsonl`, one `CostRecord` per line.
- Legacy migration: a pre-existing `<workspace>/.openhuman/costs.db` is moved (rename, copy-fallback) to the new path on first `CostTracker::new`.
- In-memory caches in `CostStorage`: `daily_cost_usd` / `monthly_cost_usd` plus the cached day/year/month they pertain to; rebuilt by full file scan on construction and on period rollover. Malformed lines are skipped with a `warn`.
- Per-session in-memory `Vec<CostRecord>` (`session_costs`) backs the session figures in `get_summary`.

## Dependencies

- `crate::openhuman::config` — `CostConfig` / `Config` (limits, warn percent, `dashboard` thresholds/currency/enabled, `workspace_dir`); `config::rpc::load_config_with_timeout` in schemas.
- `crate::openhuman::inference::provider::traits::UsageInfo` — provider usage payload translated into `TokenUsage` in `global.rs`.
- `crate::core::all` — `ControllerFuture`, `RegisteredController` for controller registration.
- `crate::core` — `ControllerSchema`, `FieldSchema`, `TypeSchema`.
- `crate::rpc::RpcOutcome` — RPC return wrapper.
- External: `chrono`, `serde`/`serde_json`, `uuid`, `parking_lot`, `once_cell`, `anyhow`, `tempfile` (tests).

## Used by

- `src/core/all.rs` — registers `all_cost_registered_controllers` / `all_cost_controller_schemas`.
- `src/core/jsonrpc.rs` — calls `cost::init_global(cfg.cost.clone(), &workspace_dir)` at bootstrap.
- `src/openhuman/agent/harness/session/turn.rs` — calls `cost::record_provider_usage` after provider calls to log per-turn usage.
- `src/openhuman/config/schema/identity_cost.rs` — `CostConfig` definition references `check_budget` / `record_provider_usage` semantics in docs.

## Notes / gotchas

- **`cost.enabled` gates enforcement only, not telemetry.** When `false`, `check_budget` returns `Allowed` and `record_usage` is a no-op, but the agent path uses `record_usage_unconditional`, so `costs.jsonl` still grows. This is a deliberate behavioural change (logged with a `warn` on init for upgraders) so spend history exists before turning on hard caps.
- The global tracker is a one-shot `OnceCell`; `init_global` is idempotent and never panics on construction failure (it logs and leaves `try_global() == None`). Callers before bootstrap (e.g. unit tests) must treat the absence as a soft no-op.
- `record_provider_usage` skips all-zero `UsageInfo` payloads (`input==0 && output==0 && charged==0.0`) so providers that don't echo usage don't inflate the request count.
- The RPC fallback tracker (`resolve_tracker`) shares the same JSONL file as the real tracker and is read-effective only; it caches by workspace path and replays a construction error for `FALLBACK_ERROR_TTL` (30s) to avoid hammering a bad workspace on the UI's ~10s poll.
- `budget_utilization` is clamped to `1.0` for display; `budget_status` is computed from the raw (unclamped) utilisation against `warn`/`alert` thresholds. A non-positive monthly limit forces `BudgetStatus::Normal` and `0.0` utilisation.
- All amounts are stored/computed in USD; `currency` is a presentation hint only.
- Time bucketing is UTC throughout (`naive_utc().date()`); model is the bucket key for per-model stats, and `provider` is derived from the `provider/model` slash prefix in DTO mapping.

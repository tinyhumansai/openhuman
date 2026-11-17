# cost

Local API-usage cost tracking for the agent. Records per-call token usage and computed USD cost to an append-only JSONL file, maintains in-memory daily/monthly aggregates, exposes a 7-day dashboard and a bounded usage log over JSON-RPC. A process-global singleton tracker is shared by the agent turn loop (telemetry) and the dashboard RPC handlers so each provider call is persisted exactly once.

## Responsibilities

- Compute per-call cost in USD from token counts and per-million prices (`TokenUsage::new`), clamping non-finite/negative prices to `0.0`.
- Preserve provider-reported usage provenance on persisted records: cached input tokens, cache-creation tokens, reasoning tokens, and whether `cost_usd` is `estimated` or `provider_charged`.
- Persist each usage event as a `CostRecord` line in `costs.jsonl` (durable: write + `sync_all`).
- Maintain cached current-day / current-month spend aggregates, rebuilt on day/month rollover.
- Capture dashboard telemetry **unconditionally** (independent of `cost.enabled`) via `record_usage_unconditional`, so history remains available to the usage view.
- Aggregate a 7-day daily history (zero-filling gap days), monthly pace projection, and per-model breakdown for the dashboard. Legacy budget fields remain on the RPC payload for compatibility, but the UI does not present them as limits.
- Serve the dashboard / daily-history / summary over JSON-RPC, with a cached read-only fallback tracker when the global is uninitialised.

## Key files

| File                                  | Role                                                                                                                                                                                                                    |
| ------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/openhuman-core/src/platform/cost/mod.rs`           | Export-focused module root; re-exports tracker, types, global helpers, and the `all_cost_*` controller schema/registry pair.                                                                                            |
| `crates/openhuman-core/src/platform/cost/types.rs`         | Serde domain types: `TokenUsage`, `CostSource`, `CostRecord`, `CostSummary`, `ModelStats`, `DailyCostEntry`, `BudgetStatus`, `CostDashboard`. Cost-calc logic lives in `TokenUsage::new`. |
| `crates/openhuman-core/src/platform/cost/tracker.rs`       | `CostTracker` (recording, summaries, daily history, dashboard build) plus the private `CostStorage` JSONL persistence + aggregate-cache layer. Functions as both `ops` and `store`.                      |
| `crates/openhuman-core/src/platform/cost/global.rs`        | Process-global `OnceCell<Arc<CostTracker>>` singleton: `init_global`, `try_global`, `record_provider_usage`, and `build_token_usage` (provider `UsageInfo` → `TokenUsage`).                                             |
| `crates/openhuman-core/src/platform/cost/rpc.rs`           | RPC-facing handlers (`dashboard`, `daily_history`, `summary`) returning `Outcome<Value>`; DTO types; `resolve_tracker` with a cached fallback tracker + error-replay TTL.                                            |
| `crates/openhuman-core/src/platform/cost/schemas.rs`       | Controller schemas + `handle_*` JSON-RPC dispatchers; `all_controller_schemas` / `all_registered_controllers`.                                                                                                          |
| `crates/openhuman-core/src/platform/cost/tracker_tests.rs` | Sibling test suite for [`tracker.rs`](./tracker.rs) (`#[path]`-included).                                                                                                                                                               |
| `crates/openhuman-core/src/platform/cost/catalog.rs` | Product configuration enrichment, source attribution, flat catalog projection, and discovered local-model overlays. Published prices, context windows, ID matching, and estimation belong to [`tinyagents_registry::pricing`](https://github.com/tinyhumansai/tinyagents/blob/a55f25a924a673c812c7aa95e562488dd0d0f3b3/crates/tinyagents-registry/src/pricing/README.md). |
| `crates/openhuman-core/src/platform/cost/route.rs` | `CostRoute` / `route_for_model`: derives from the model id whether a record counts against OpenHuman-managed credits or is BYOK/local (#5016). |
| `crates/openhuman-core/src/platform/cost/scope.rs` | `UsageScope::ambient`: a record's attribution (thread, origin, agent definition, sub-agent task, embedded/SaaS user agent, provider) read from the recording task's turn origin, memory identity and `CoreContext`. |
| `crates/openhuman-core/src/platform/cost/report.rs` | Pure usage reports over ledger records: `build_report` (group by day/week/month/model/provider/route/agent/thread/origin/session_agent; tokens, charged vs estimated cost, cache-hit ratio) and `build_cache_report` (per-call hits, cold calls, uncached premium). |
| `crates/openhuman-core/src/platform/cost/budget.rs` | Budgets (`[[cost.budgets]]`): `evaluate` / `check_call` sum a policy's bucket (global, or this call's thread/agent/model/user agent) over its day or month and report warnings and refusals. The agent budget gate (`agent/tinyagents/host/budget_gate.rs`) runs it before every model call. |
| `crates/openhuman-core/src/platform/cost/tools.rs` | Read-only, default-on LLM tools (`cost_get_dashboard`, `cost_get_daily_history`, …) re-exported through `crates/openhuman-core/src/tools/mod.rs`. |

## Public surface

From [`mod.rs`](./mod.rs) re-exports:

- `CostTracker`: the tracker (`tracker`).
- `init_global`, `rebind_global`, `try_global`, `record_provider_usage` (`global`).
- `all_cost_controller_schemas`, `all_cost_registered_controllers` (`schemas`).
- Types: `BudgetStatus`, `CostDashboard`, `CostRecord`, `CostSource`, `CostSummary`, `DailyCostEntry`, `ModelStats`, `TokenUsage`.

Notable `CostTracker` methods: `new`, `session_id`, `record_usage`, `record_usage_unconditional`, `get_summary`, `get_daily_cost`, `get_monthly_cost`, `get_daily_history`, `get_dashboard`.

## RPC / controllers

Namespace `cost` (methods `openhuman.cost_*` via the registry):

| Method                   | Inputs                                       | Output                                                                                                     |
| ------------------------ | -------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `cost_get_dashboard`     | none                                         | 7-day dashboard payload: per-day buckets, summary metrics, legacy budget fields, per-model breakdown. |
| `cost_get_daily_history` | `days?` (u32, default 7, clamped `[1, 366]`) | Ordered daily entries, oldest first, gaps zero-filled.                                                     |
| `cost_get_summary`       | none                                         | Live session / daily / monthly cost summary.                                                               |
| `cost_get_usage_log`     | `days?`, `limit?`                            | Recent local records, newest first, bounded to 1,000 rows.                                                |
| `cost_report`            | `days?`, `groupBy?`, `filter?`               | Totals plus one row per group (most expensive first): calls, input/output/cached/cache-write/reasoning tokens, `cost_usd` split into `charged_usd` / `estimated_usd`, `cache_hit_ratio`. |
| `cost_cache_report`      | `days?`, `filter?`, `limit?`                 | Per-call prompt-cache hits (newest `limit`), overall `cache_hit_ratio`, `cold_calls` (a repeat call in a thread with no cache read) and `uncached_premium_usd`. |

Both reports are also CLI commands: `openhuman-core cost report --days 7 --groupBy '["day","agent"]'`, `openhuman-core cost cache_report --filter '{"thread_id":"…"}'`.

**Attribution.** Every recorded call carries an optional `scope` (`UsageScope`): the thread, what started the turn, the agent definition (or the delegated sub-agent and its task), the embedded/SaaS user agent and the provider. The event bridge passes the provider and sub-agent, and the rest comes from the recording task. Records written before attribution have no `scope` and group as `unknown`.

Handlers load config via `config_rpc::load_config_with_timeout`, then delegate to [`rpc.rs`](./rpc.rs). RPC DTOs (`CostDashboardDto`, `DailyCostEntryDto`, `ModelStatsDto`, `CostSummaryDto`, `UsageLogRecordDto`) add presentation fields not on the domain types: `provider` (derived from the `provider/model` prefix), `percent_of_total`, and dashboard threshold/`enabled` flags from `cost.dashboard`. Usage-log records preserve the persisted token provenance fields (`cached_input_tokens`, `cache_creation_tokens`, `reasoning_tokens`, `cost_source`) for migration audit callers for the dedicated usage-log tab.

## Events

None. The module has no `bus.rs` and no `DomainEvent` publishers/subscribers.

## Persistence

- Append-only JSONL at `<workspace>/state/costs.jsonl`, one `CostRecord` per line. When the host configured a storage backend (`crate::storage`), records are `cost_records` documents under the acting agent's scope instead ([`tracker_documents.rs`](./tracker_documents.rs)), and the file is not used; the period totals are recomputed from the scope's documents rather than cached.
- Legacy migration: a pre-existing `<workspace>/.openhuman/costs.db` is moved (rename, copy-fallback) to the new path on first `CostTracker::new`.
- In-memory caches in `CostStorage`: `daily_cost_usd` / `monthly_cost_usd` plus the cached day/year/month they pertain to; rebuilt by full file scan on construction and on period rollover. Malformed lines are skipped with a `warn`.
- Per-session in-memory `Vec<CostRecord>` (`session_costs`) backs the session figures in `get_summary`.

## Dependencies

- `crate::config`: `CostConfig` / `Config` (legacy display target, dashboard currency/enabled, `workspace_dir`); `config::rpc::load_config_with_timeout` in schemas.
- `tinyinference_llm::usage::BilledUsage`: provider usage payload translated into `TokenUsage` in [`global.rs`](./global.rs).
- `crate::core::all`: `ControllerFuture`, `RegisteredController` for controller registration.
- `crate::core`: `ControllerSchema`, `FieldSchema`, `TypeSchema`.
- `crate::core::Outcome`: RPC return wrapper.
- External: `chrono`, `serde`/`serde_json`, `uuid`, `parking_lot`, `once_cell`, `anyhow`, `tempfile` (tests).

## Used by

- `crates/openhuman-core/src/core/all.rs`: registers `all_cost_registered_controllers` / `all_cost_controller_schemas`.
- `crates/openhuman-core/src/core/runtime/subscribers.rs`: calls `cost::init_global(cfg.cost.clone(), &workspace_dir)` at bootstrap.
- `crates/openhuman-core/src/agent/tinyagents/observability/event_bridge.rs`, `agent/tinyagents/turn_outcome.rs`, `agent/tinyagents/host/budget_gate.rs`, and `agent/subagent_host/`: call `cost::record_provider_usage` after provider calls to log per-turn (and subagent) usage.
- `crates/openhuman-core/src/tools/mod.rs`: re-exports `platform::cost::tools::*`.
- `crates/openhuman-core/src/config/schema/identity_cost.rs`: `CostConfig` retains legacy budget-display fields for wire compatibility.

## Notes / gotchas

- **`cost.enabled` gates `record_usage`, not telemetry.** The agent path uses `record_usage_unconditional`, so the local usage ledger continues to grow when this flag is false. The legacy `monthly_limit_usd` only drives the dashboard; the only caps the core enforces are the opt-in `[[cost.budgets]]` below. Hosted-credit exhaustion is enforced by the backend.
- The global tracker is a one-shot `OnceCell`; `init_global` is idempotent and never panics on construction failure (it logs and leaves `try_global() == None`). Callers before bootstrap (e.g. unit tests) must treat the absence as a soft no-op.
- `record_provider_usage` skips all-zero `UsageInfo` payloads (`input==0 && output==0 && charged==0.0`) so providers that don't echo usage don't inflate the request count.
- Provider-charged USD is persisted directly with `cost_source = provider_charged`; otherwise usage remains `estimated`. Cached input tokens are clamped to `input_tokens` during provider usage translation.
- The RPC fallback tracker (`resolve_tracker`) shares the same JSONL file as the real tracker and is read-effective only; it caches by workspace path and replays a construction error for `FALLBACK_ERROR_TTL` (30s) to avoid hammering a bad workspace on the UI's ~10s poll.
- Legacy `budget_utilization` is clamped to `1.0` in the RPC payload; `budget_status` is computed from the raw (unclamped) utilisation against `warn`/`alert` thresholds. A non-positive monthly limit forces `BudgetStatus::Normal` and `0.0` utilisation.
- All amounts are stored/computed in USD; `currency` is a presentation hint only.
- Time bucketing is UTC throughout (`naive_utc().date()`); model is the bucket key for per-model stats, and `provider` is derived from the `provider/model` slash prefix in DTO mapping.

## Budgets

Budgets are opt-in and empty by default:

```toml
[[cost.budgets]]
name = "monthly cap"
max_usd = 50.0           # and/or max_tokens
period = "month"         # or "day" (UTC)
action = "refuse"        # or "warn" (default)

[[cost.budgets]]
name = "planner per day"
scope = "agent"          # global (default) | thread | agent | model | session_agent
match = "planner"        # omit to apply to each value separately
period = "day"
max_usd = 2.0
warn_fraction = 0.8      # default
```

Before every model call, the agent's budget gate (`OpenHumanBudgetGate::acquire`) sums each policy's bucket from the ledger over its period:

- **Refuse:** a `refuse` policy at or over its limit refuses the call with `TinyAgentsError::LimitExceeded("BUDGET_EXCEEDED: …")`, before any scheduler slot is taken.
- **Warn:** a `warn` policy, or any policy past `warn_fraction`, logs a warning.
- **Unattributed calls:** a call without the policy's attribute is outside it. For example, a call with no thread is outside a per-thread budget.
- **The call itself counts:** its estimated cost and tokens are added to the bucket's total, so a call that would itself cross a limit is refused.
- **Invalid caps fail closed:** a negative or NaN `max_usd` refuses (with a warning). A NaN `warn_fraction` uses 0.8, and records with a non-finite cost count as free.
- **Live policies:** the gate re-reads `[[cost.budgets]]` from the session's `config.toml` on every check, so a change applies to threads already open. It keeps the policies it was built with when the file is missing or does not parse (logged at warn), and always for an embedder-supplied config, whose in-memory budgets are authoritative.
- **Soft cap under concurrency:** the check reads the ledger and reserves nothing. Calls that start together can each pass, and the overshoot is bounded by the estimates of the calls in flight. Spend is known once a call is recorded; a provider reply with no usage is not recorded and so does not count.
- **Agent calls only:** the gate covers model calls made through the agent harness. Direct `ChatModel` callers (chat follow-up suggestions, Flow Canvas LLM nodes) are not metered against budgets yet.
- **No `provider` scope yet:** the gate does not see a call's provider before it is made.
- **No checking at all** when no budgets are configured, when there is no cost tracker, or when the ledger cannot be read. The budget check never fails a call for a reason of its own.

## Further reading

- [Parent module (`platform`)](../README.md)
- [Billing and usage](../../../../../docs/gitbooks/en/features/billing-and-usage.md)
- [Platform and availability](../../../../../docs/gitbooks/en/features/platform.md)

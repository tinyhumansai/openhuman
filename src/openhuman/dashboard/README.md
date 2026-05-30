# dashboard

Aggregate, operator-facing views over local config. Today it owns a single read-only view: the per-model health comparison table rendered in the desktop **Settings → Developer Options → Model Health** panel. The view joins the local `Config::model_registry` with the `dashboard.model_health` thresholds and emits one row per model. Telemetry-driven metric fields (`quality_score`, `hallucination_rate`, `agents_using`, `tasks_evaluated`) are intentional placeholders (`null` / `0`) until a local telemetry pipeline lands — the placeholder contract is documented in `ops.rs` and asserted in its tests. Stateless: no persistence, no event-bus subscribers, no agent tools.

## Responsibilities

- Build the model health comparison response by mapping each `model_registry` entry to a `ModelHealthEntry` (id, provider, cost per 1M output tokens, vision flag).
- Surface the `dashboard.model_health` thresholds (`hallucination_threshold`, `min_tasks_for_rating`, `evaluation_window_tasks`) so the frontend can compute status badges.
- Emit telemetry metric fields as explicit placeholders (`None` / `0`) — to be populated here, not at the transport layer, once telemetry exists.
- Reject the request with an error when `dashboard.model_health.enabled` is `false`.
- Expose the view over JSON-RPC via the controller registry.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/dashboard/mod.rs` | Export-only: module docstring + `mod`/`pub use` re-exports of ops, schemas, and types. |
| `src/openhuman/dashboard/types.rs` | Wire types: `ModelHealthEntry`, `ModelHealthConfigView`, `ModelHealthResponse` (serde). |
| `src/openhuman/dashboard/ops.rs` | Business logic: `model_health(&Config)` joins registry + thresholds, returns `RpcOutcome<ModelHealthResponse>`. Inline tests cover mapping, thresholds, disabled feature, empty registry. |
| `src/openhuman/dashboard/schemas.rs` | Controller schemas + registered controller + `handle_dashboard_model_health` handler (loads config via timeout, delegates to `ops::model_health`). Inline tests assert schema stability and list-length parity. |

## Public surface

- `ops::model_health` — `fn model_health(config: &Config) -> Result<RpcOutcome<ModelHealthResponse>, String>`.
- `schemas::all_dashboard_controller_schemas`, `schemas::all_dashboard_registered_controllers`, `schemas::dashboard_schemas` — controller-registry entry points.
- `types::ModelHealthEntry`, `types::ModelHealthConfigView`, `types::ModelHealthResponse`.

## RPC / controllers

| Method | Inputs | Outputs |
| --- | --- | --- |
| `openhuman.dashboard_model_health` (namespace `dashboard`, function `model_health`) | none | `models: ModelHealthEntry[]`, `config: ModelHealthConfigView` |

`ModelHealthEntry`: `id`, `provider`, `cost_per_1m_output` (f64), `vision` (bool), `quality_score` (`f64?`, placeholder), `hallucination_rate` (`f64?`, placeholder), `agents_using` (u64, placeholder 0), `tasks_evaluated` (u64, placeholder 0).
`ModelHealthConfigView`: `hallucination_threshold` (f64), `min_tasks_for_rating` (u64), `evaluation_window_tasks` (u64).

The handler loads config via `crate::openhuman::config::rpc::load_config_with_timeout()` and returns CLI-compatible JSON through `RpcOutcome::into_cli_compatible_json()`. Wired into the registry in `src/core/all.rs` (both `all_dashboard_registered_controllers` and `all_dashboard_controller_schemas`).

## Persistence

None. The module reads from in-memory `Config`; it stores no state.

## Dependencies

- `crate::openhuman::config` (`Config`, `config::rpc::load_config_with_timeout`) — source of the model registry and `dashboard.model_health` thresholds. `DashboardConfig` / `ModelHealthConfig` are defined in `src/openhuman/config/schema/dashboard.rs`.
- `crate::core::all` (`ControllerFuture`, `RegisteredController`) and `crate::core` (`ControllerSchema`, `FieldSchema`, `TypeSchema`) — controller registry types.
- `crate::rpc::RpcOutcome` — standard RPC result wrapper.
- `serde` / `serde_json` — wire (de)serialization and handler params.

## Used by

- `src/core/all.rs` — registers the controller and its schema into the global RPC/CLI registry (lines ~119 and ~297).

## Notes / gotchas

- **Placeholder contract is load-bearing.** `quality_score` / `hallucination_rate` are always `None` and `agents_using` / `tasks_evaluated` always `0`. The frontend treats null quality/hallucination as "no signal", collapsing badges to `staging`. When telemetry lands, populate these in `ops::model_health`, not in the transport layer.
- Disabled feature (`dashboard.model_health.enabled == false`) returns `Err("model health disabled")`, which the handler surfaces as an RPC error.
- This is the only view in the domain so far; `mod.rs` is intentionally export-only per the canonical module shape (no `store.rs`, `tools.rs`, or `bus.rs` — the domain is stateless, owns no agent tools, and has no event subscribers).

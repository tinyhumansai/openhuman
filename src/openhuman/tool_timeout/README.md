# tool_timeout

Process-wide wall-clock timeout policy for tool execution (the node/tool runtime and the agent loop). It resolves a single bounded timeout value and exposes it as seconds and as a `Duration` for callers that wrap individual tool calls in a timeout. The value is **runtime-mutable**: the UI (via the `config.update_agent_settings` RPC) can change it without a core restart, and the change takes effect on the next tool call.

## Resolution order

Highest precedence first:

1. `OPENHUMAN_TOOL_TIMEOUT_SECS` environment variable — operator override. When set to a valid value (`1..=3600`) it always wins; config pushes are ignored while it is present.
2. The persisted config value (`[agent].agent_timeout_secs`), pushed in via `set_tool_timeout_secs` at startup and on every `config.update_agent_settings` RPC.
3. The built-in `DEFAULT_TIMEOUT_SECS` (`120`) default.

## Responsibilities

- Hold the effective timeout in a process-global `AtomicU64`, seeded lazily from env/default on first read.
- Bound every candidate value to `1..=3600` seconds; fall back to the `120`s default on missing, non-numeric, zero, negative, or out-of-range input.
- Let the persisted config drive the value at runtime while keeping the operator env var as an always-wins override.
- Provide the timeout to callers in two shapes: raw seconds (for logging / matching frontend timeouts) and `Duration` (for `tokio::time::timeout`-style wrapping).
- Keep parsing/resolution logic pure and testable, isolated from global-state mutation.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/tool_timeout/mod.rs` | Entire module: constants, env parsing, pure resolver, atomic-backed runtime value, setter, public accessors, and inline unit tests. |

## Public surface

- `parse_tool_timeout_secs(raw: Option<&str>) -> u64` — pure parser; bounds to `1..=3600`, else returns the `120`s default.
- `set_tool_timeout_secs(config_secs: u64) -> u64` — push a config-sourced value into the runtime atomic, honouring the env override. Returns the effective value stored. Called at startup and on each config update.
- `env_override_active() -> bool` — `true` when `OPENHUMAN_TOOL_TIMEOUT_SECS` is set to a valid override (so UI changes are ignored). Surfaced to the settings panel.
- `tool_execution_timeout_secs() -> u64` — effective timeout in seconds (read fresh each call).
- `tool_execution_timeout_duration() -> Duration` — same effective value as a `Duration`.
- `explicit_call_timeout_secs(requested: Option<u64>, cap: u64) -> Option<u64>` — resolve an **explicit** per-call timeout for an otherwise-unbounded scripting tool. `None`/`Some(0)` ⇒ `None` (run unbounded); any positive value clamps to `MIN_TIMEOUT_SECS..=cap`. Callers pass their own ceiling (`MAX_TIMEOUT_SECS` for `shell`, `1800` for `node_exec`/`npm_exec`).
- `explicit_call_timeout_duration(requested: Option<u64>, cap: u64) -> Option<Duration>` — same as a `Duration`, `None` for unbounded.
- Constants: `DEFAULT_TIMEOUT_SECS = 120`, `MIN_TIMEOUT_SECS = 1`, `MAX_TIMEOUT_SECS = 3600`, `SANDBOX_UNBOUNDED_CAP_SECS = 86_400`, `ENV_VAR = "OPENHUMAN_TOOL_TIMEOUT_SECS"`.

## Scripting tools run unbounded (issue #4023)

The global timeout governs **non-scripting** tools only — a hung network/MCP call must stay bounded. The scripting tools (`shell`, `node_exec`, `npm_exec`) instead run with **no** default deadline: a build / solver / test run legitimately takes minutes and must not be hard-killed. They expose a per-call `timeout_secs` argument and return [`Tool::timeout_policy`] → `ToolTimeout::Unbounded` when none is supplied, or `ToolTimeout::Secs(n)` when one is. The harness (`agent/harness/engine/tools.rs::resolve_tool_deadline`) maps `Unbounded` to "no `tokio::time::timeout` wrapper at all" and `Secs(n)` to a clamped deadline plus a small grace margin so the tool's own internal timeout (which actually kills the child) fires first. Sandbox backends, which require a finite deadline, substitute `SANDBOX_UNBOUNDED_CAP_SECS` (24h) for the unbounded case.

## Configuration

- `[agent].agent_timeout_secs` (config TOML) — integer seconds, valid range `1..=3600`, default `120`. Editable live via **Settings → Agent OS access → Action timeout** or the `config.update_agent_settings` RPC.
- `OPENHUMAN_TOOL_TIMEOUT_SECS` (env) — operator override with the same range. When valid it overrides the config value; an invalid value is ignored so the config value still applies.

## Dependencies

- `log` for the debug trace on config pushes. Otherwise only `std` (`std::sync::atomic::AtomicU64`, `std::time::Duration`, `std::env`).

## Used by

- `src/openhuman/agent/harness/engine/tools.rs` — `run_one_tool` wraps each tool `execute()` according to its [`Tool::timeout_policy`] (via `resolve_tool_deadline`): `Inherit` uses `tool_execution_timeout_secs()`; `Secs(n)` uses a clamped value plus grace; `Unbounded` runs with no deadline. This is the single per-tool-call enforcement point, reached by the main loop, the session executor, and the sub-agent inner loop.
- `src/openhuman/tools/impl/system/{shell,node_exec,npm_exec}.rs` — scripting tools: unbounded by default, explicit `timeout_secs` via `explicit_call_timeout_*`.
- `src/openhuman/agent/tools/delegate.rs` — bounds the delegated provider chat call with `tool_execution_timeout_secs`.
- `src/openhuman/config/ops.rs` — `apply_agent_settings` calls `set_tool_timeout_secs` after persisting; `get_agent_settings` reports `effective_timeout_secs` / `env_override`.
- `src/openhuman/channels/runtime/startup.rs` — seeds the runtime value from config at core boot.
- `src/openhuman/agent/harness/harness_gap_tests.rs` — pins `parse_tool_timeout_secs` default/boundary behaviour.

## Notes / gotchas

- The value is read fresh on every tool call, so a config change takes effect on the **next** tool call. A `tokio::time::timeout` already in flight keeps the deadline it captured.
- `0` is deliberately rejected (it would mean "disable timeout") and falls back to the default rather than disabling.
- A present-but-invalid env value (non-numeric / `0` / out of range) counts as "no override", so the config value still applies — only a valid env value overrides.
- The default (`120`s) must stay in sync with any frontend timeout that mirrors it (`app/src/utils/config.ts` `TOOL_TIMEOUT_SECS`).

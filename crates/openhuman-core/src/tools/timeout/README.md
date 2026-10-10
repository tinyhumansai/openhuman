# tool_timeout

Process-wide wall-clock timeout policy for tool execution (the node/tool runtime and the agent loop). It resolves a single bounded timeout value and exposes it as seconds and as a `Duration` for callers that wrap individual tool calls in a timeout. The value is runtime-mutable: the UI (via the `config.update_agent_settings` RPC) can change it without a core restart, and the change takes effect on the next tool call.

## Resolution order

Highest precedence first:

1. `OPENHUMAN_TOOL_TIMEOUT_SECS` environment variable, an operator override. When set to a valid value (`1..=3600`) it always wins; config pushes are ignored while it is present.
2. The persisted config value (`[agent].agent_timeout_secs`), pushed in via `set_tool_timeout_secs` at startup (from `core::runtime::subscribers::register_domain_subscribers`, the always-on core boot path) and on every `config.update_agent_settings` RPC.
3. The built-in `DEFAULT_TIMEOUT_SECS` (`120`) default.

## Responsibilities

- Hold the effective timeout in a process-global vendored `tinyagents_harness::tool::ToolTimeoutSettings` (atomic inside), seeded lazily from env or default on first read. Env/config parsing stays here.
- Bound every candidate value to `1..=3600` seconds, falling back to the `120`s default on missing, non-numeric, zero, negative, or out-of-range input.
- Let the persisted config drive the value at runtime while keeping the operator env var as an always-wins override.
- Provide the timeout to callers in two shapes: raw seconds (for logging and matching frontend timeouts) and `Duration` (for `tokio::time::timeout`-style wrapping).
- Keep parsing and resolution logic pure and testable, isolated from global-state mutation.

## Key files

| File | Role |
| --- | --- |
| `crates/openhuman-core/src/tools/timeout/mod.rs` | Entire module: constants, env parsing, pure resolver, atomic-backed runtime value, setter, public accessors. |
| `crates/openhuman-core/src/tools/timeout/mod_tests.rs` | Unit tests for the module, run via the `#[path = "mod_tests.rs"] mod tests` include in [`mod.rs`](./mod.rs). |

## Public surface

- `parse_tool_timeout_secs(raw: Option<&str>) -> u64`: pure parser. Bounds to `1..=3600`, else returns the `120`s default.
- `set_tool_timeout_secs(config_secs: u64) -> u64`: pushes a config-sourced value into the runtime atomic, honoring the env override. Returns the effective value stored. Called at startup and on each config update.
- `env_override_active() -> bool`: true when `OPENHUMAN_TOOL_TIMEOUT_SECS` is set to a valid override, so UI changes are ignored. Surfaced to the settings panel.
- `tool_execution_timeout_secs() -> u64`: effective timeout in seconds, read fresh each call.
- `install_harness_tool_timeouts(harness)`: installs the shared settings on a TinyAgents `AgentHarness` (`with_tool_timeout_settings`). The harness enforces `ToolTimeout` policies only when settings are installed; the turn harness (`agent/tinyagents/harness_assembly.rs`) and the live voice harness call this. Tools that wait on a human inside `execute` carry their own budgets (`composio_connect`: park bound + 60s; `browser`: 12 min).
- `explicit_call_timeout_secs(requested: Option<u64>, cap: u64) -> Option<u64>`: resolves an explicit per-call timeout for an otherwise-unbounded scripting tool. `None` or `Some(0)` means `None` (run unbounded); any positive value clamps to `MIN_TIMEOUT_SECS..=cap`. Callers pass their own ceiling (`MAX_TIMEOUT_SECS` for `shell`, the shell limit for shell).
- `explicit_call_timeout_duration(requested: Option<u64>, cap: u64) -> Option<Duration>`: same as a `Duration`, `None` for unbounded.
- `resolve_tool_deadline(policy: tinytools::ToolTimeout) -> (Option<Duration>, u64)`: resolves a tool's `ToolTimeout` policy (`Inherit`, `Millis(req)`, or `Unbounded`) into the `(deadline, timeout_secs)` pair the agent tool-execution loop enforces, padding an explicit millisecond request with `TOOL_TIMEOUT_GRACE_SECS` before the hard deadline fires.
- Constants: `DEFAULT_TIMEOUT_SECS = 120`, `MIN_TIMEOUT_SECS = 1`, `MAX_TIMEOUT_SECS = 3600`, `SANDBOX_UNBOUNDED_CAP_SECS = 86_400`, `ENV_VAR = "OPENHUMAN_TOOL_TIMEOUT_SECS"`.

## Scripting tools run unbounded (issue #4023)

The global timeout governs non-scripting tools only, since a hung network or MCP call must stay bounded. The scripting tools (`shell`) instead run with no default deadline: a build, solver, or test run legitimately takes minutes and must not be hard-killed. They expose a per-call `timeout_secs` argument and resolve it through `explicit_call_timeout_secs`/`explicit_call_timeout_duration`: `None` runs unbounded, a positive value clamps to the tool's own ceiling. Sandbox backends, which require a finite deadline even when the caller asked for none, substitute `SANDBOX_UNBOUNDED_CAP_SECS` (24h) for the unbounded case.

## Configuration

- `[agent].agent_timeout_secs` (config TOML): integer seconds, valid range `1..=3600`, default `120`. Editable live via Settings, Agent OS access, Action timeout, or the `config.update_agent_settings` RPC.
- `OPENHUMAN_TOOL_TIMEOUT_SECS` (env): operator override with the same range. When valid it overrides the config value; an invalid value is ignored so the config value still applies.

## Dependencies

- `log` for the debug trace on config pushes. Otherwise only `std` (`std::sync::atomic::AtomicU64`, `std::time::Duration`, `std::env`).

## Used by

- `crates/openhuman-core/src/agent/tinyagents/tools.rs`: OpenHuman tools execute through `execute_with_options`, which applies each tool's `Tool::timeout_policy`.
- `crates/openhuman-core/src/tools/impl/system/shell.rs`: the shell tool, unbounded by default, with explicit `timeout_secs` via `explicit_call_timeout_*`.
- `crates/openhuman-core/src/agent/tools/delegate.rs`: bounds the delegated provider chat call with `tool_execution_timeout_secs`.
- `crates/openhuman-core/src/config/ops/agent.rs`: `apply_agent_settings` calls `set_tool_timeout_secs` after persisting; `get_agent_settings` reports `effective_timeout_secs`/`env_override`.
- `crates/openhuman-core/src/core/runtime/subscribers.rs`: `register_domain_subscribers` seeds the runtime value from config on the always-on core boot path, so channel-less or web-chat-only cores get the configured timeout too (#5027).

## Notes and gotchas

- The value is read fresh on every tool call, so a config change takes effect on the next tool call. A `tokio::time::timeout` already in flight keeps the deadline it captured.
- `0` is deliberately rejected (it would mean "disable timeout") and falls back to the default rather than disabling.
- A present-but-invalid env value (non-numeric, `0`, or out of range) counts as "no override", so the config value still applies. Only a valid env value overrides.
- The default (`120`s) must stay in sync with any frontend timeout that mirrors it (`app/src/utils/config.ts` `TOOL_TIMEOUT_SECS`).

## Further reading

- [Parent module (`tools`)](../README.md)
- [Native tools overview](../../../../../gitbooks/features/native-tools/README.md)
- [Agent harness architecture](../../../../../gitbooks/developing/architecture/agent-harness.md)
- [Approval gate](../../../../../gitbooks/features/approval-gate.md)
- [tinyagents submodule](../../../../../vendor/tinyagents/README.md)

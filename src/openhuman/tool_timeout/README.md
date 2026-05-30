# tool_timeout

Process-wide wall-clock timeout policy for tool execution (the node/tool runtime and the agent loop). It resolves a single bounded timeout value once per process from the `OPENHUMAN_TOOL_TIMEOUT_SECS` environment variable and exposes it as seconds and as a `Duration` for callers that wrap individual tool calls in a timeout. There is no per-tool or per-session override — it is one global, immutable value.

## Responsibilities

- Read `OPENHUMAN_TOOL_TIMEOUT_SECS` once and cache it (via `OnceLock`).
- Bound the value to `1..=3600` seconds; fall back to the `120`s default on missing, non-numeric, zero, negative, or out-of-range input.
- Provide the timeout to callers in two shapes: raw seconds (for logging / matching frontend timeouts) and `Duration` (for `tokio::time::timeout`-style wrapping).
- Keep parsing logic pure and testable, isolated from global-state resolution.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/tool_timeout/mod.rs` | Entire module: constants, env parsing, cached resolution, public accessors, and inline unit tests. |

## Public surface

- `parse_tool_timeout_secs(raw: Option<&str>) -> u64` — pure parser; bounds to `1..=3600`, else returns the `120`s default. Split out so unit tests avoid racing on `OnceLock` or mutating the process env.
- `tool_execution_timeout_secs() -> u64` — resolved timeout in seconds (cached). Used for logging and matching frontend timeouts.
- `tool_execution_timeout_duration() -> Duration` — same resolved value as a `Duration`.

Internal (not exported): `DEFAULT_SECS = 120`, `MAX_SECS = 3600`, `ENV_VAR = "OPENHUMAN_TOOL_TIMEOUT_SECS"`, and `resolved_secs()` which lazily initializes the `OnceLock<u64>`.

## Configuration

- `OPENHUMAN_TOOL_TIMEOUT_SECS` — integer seconds, valid range `1..=3600`. Anything outside that (missing, non-numeric, `0`, negative, `> 3600`) resolves to the `120`s default. Read directly from `std::env::var`, not through the TOML `Config` struct. Resolved once per process; subsequent env changes have no effect.

## Dependencies

- None on other `openhuman`/`core` modules. Uses only `std` (`std::sync::OnceLock`, `std::time::Duration`, `std::env`).

## Used by

- `src/openhuman/agent/tools/delegate.rs` — imports `tool_execution_timeout_secs`.
- `src/openhuman/agent/harness/tool_loop.rs` — uses both `tool_execution_timeout_duration` (to bound a tool call) and `tool_execution_timeout_secs` (for logging).
- `src/openhuman/agent/harness/subagent_runner/ops.rs` — uses `tool_execution_timeout_duration`.
- `src/openhuman/agent/harness/harness_gap_tests.rs` — pins `parse_tool_timeout_secs` default/boundary behavior.

## Notes / gotchas

- The value is cached in an `OnceLock` on first read, so it is effectively immutable for the process lifetime — changing the env var at runtime does nothing.
- `0` is deliberately rejected (it would mean "disable timeout") and falls back to the default rather than disabling.
- No RPC controllers, agent tools, event-bus subscribers, or persisted state — this is a pure policy/config helper, intentionally not a full domain with `types.rs`/`ops.rs`/`schemas.rs`.
- The default (`120`s) is documented in the module docstring; keep it in sync with any frontend timeout that must match.

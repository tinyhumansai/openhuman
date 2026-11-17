# tool_timeout (host adapter)

Bounds, resolution, process-group capture (`output_or_kill`, `output_unbounded`, `output_with_input`), `ProcessCleanup` and `CommandEnvironment` live in `tinyagents_harness_tools::exec::timeout` (tinyhumansai/tinyagents). Callers import them from there directly.

This module keeps only host policy:

- `ENV_VAR` (`OPENHUMAN_TOOL_TIMEOUT_SECS`) operator override, always wins over config.
- `set_tool_timeout_secs` pushes the persisted `[agent].agent_timeout_secs` into the module settings and the harness settings.
- `install_harness_tool_timeouts` installs the harness-side `ToolTimeoutSettings` (sharing the atomic).
- A task wrapper registered with the module so its waiter tasks keep `CoreContext` and memory identity.
- `tool_execution_timeout_secs`, `env_override_active` read-outs for RPC/UI.

# Monitor

`openhuman::monitor` owns background command monitors for agent sessions.

The module is deliberately separate from `tools/impl/system`: system shell
execution remains the shared primitive, while monitor lifecycle, bounded output,
event publication, and tool/RPC contracts live here.

## Contract

- Commands use the same `SecurityPolicy` classification, gated-command check,
  rate limiter, action directory, safe environment, permission level, and audit
  channel expectations as execute-class shell tools.
- `monitor` starts a background subprocess and streams stdout/stderr line by
  line into a bounded workspace file under `<workspace>/monitor/`.
- Recent structured events are kept in process memory for `monitor_list`; full
  output is read through `monitor_read` without putting raw logs into model
  history.
- When a monitor is started inside an active agent turn, each line is queued as
  `collect` context through the current turn's run queue, so the engine injects
  it only at a safe iteration boundary.
- `monitor_stop` sends a stop signal to the runner and kills the child process.
  Timeout and natural completion update the same store and publish lifecycle
  events.

Persistent monitors are represented by metadata today; app shutdown cleanup is
handled by process teardown, while explicit stop is the reliable user-visible
control.

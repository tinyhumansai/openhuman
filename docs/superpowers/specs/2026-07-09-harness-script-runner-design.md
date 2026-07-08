
# harness_script — Phase 1: Python runner skeleton

**Issue:** [tinyhumansai/openhuman#4250](https://github.com/tinyhumansai/openhuman/issues/4250) — *Add harness_script/python_runner for LLM-authored orchestration flows*

**Status:** design approved, Phase 1 scope only.

## Scope

Issue #4250 proposes a six-phase subsystem that lets an LLM author expressive
Python orchestration scripts over OpenHuman's existing harness primitives. This
spec covers **Phase 1 (Runner skeleton) only**. It deliberately does **not**
implement:

- the orchestration bridge (`agent.run`, `workflow.run`, `checkpoint.put`, …) — Phase 2
- the `run_harness_script` / `await_harness_script` tool surface — Phase 3
- static AST validation, import/call sandboxing, approval gating — Phase 4
- observability/UI and privacy-safe graph telemetry — Phase 5
- docs/examples — Phase 6

Phase 1 exists to prove the process-and-protocol foundation the later phases
build on, and to make it independently reviewable and testable.

## Goal

Add a `src/openhuman/harness_script/` module with a
`PythonHarnessScriptRunner` that:

1. resolves a managed Python interpreter through the existing
   `runtime_python::PythonBootstrap` (no new distribution mechanism),
2. launches a one-shot subprocess with piped stdio via the existing
   `spawn_stdio` helper,
3. performs a versioned, newline-delimited-JSON handshake,
4. runs a single script to a terminal result/error under wall-time and
   output-byte caps with cooperative cancellation, and
5. deterministically kills and reaps the child on completion, timeout, or
   cancellation.

## Non-goals (Phase 1)

- Any real side-effecting capability inside the script. The Phase 1 bootstrap
  executes the script **unsandboxed** purely to exercise the protocol; this is
  marked `# PHASE 4` in `bootstrap.py`. No orchestration handlers are wired, so
  a Phase 1 script cannot spawn agents or workflows.
- Durable run ledger / storage (Phase 2+).

## Architecture

```
src/openhuman/harness_script/
  mod.rs            module doc + public re-exports
  protocol.rs       serde message types + newline-JSON framing (PROTOCOL_VERSION = 1)
  runner.rs         PythonHarnessScriptRunner, ScriptRunSpec, ScriptRunOutcome, HarnessScriptError
  bootstrap.py      embedded via include_str!; child-side protocol loop
  runner_tests.rs   #[cfg(test)] #[path = "runner_tests.rs"] mod tests;
```

Registered as `pub mod harness_script;` in `src/openhuman/mod.rs`.

Conventions mirror `runtime_python`: `anyhow` at fallible boundaries, a typed
`thiserror` error enum for the runner outcome, tracing logs prefixed
`[harness_script::…]`, and co-located `_tests.rs`.

## Protocol

Newline-delimited JSON over the child's stdin/stdout (matching the MCP stdio
pattern already used by `runtime_python::process`). stderr is treated as a
free-form log channel. Every message carries a `type` tag.

| Direction     | Message  | Fields                                             |
| ------------- | -------- | -------------------------------------------------- |
| parent → child | `init`   | `protocol_version`, `script`, `inputs`, `limits`  |
| child → parent | `ready`  | `protocol_version`                                 |
| child → parent | `result` | `output` (arbitrary JSON)                          |
| child → parent | `error`  | `error_class`, `message`, `traceback`              |

Sequence: child emits `ready` on startup → parent validates
`protocol_version` and sends `init` → child `exec`s the script and emits
exactly one terminal `result` or `error`. The gap between `ready` and the
terminal message is where Phase 2 will insert `call`/`return` request frames;
the framing is versioned so that extension does not break Phase 1 peers.

## Data flow

`run(spec, cancel)`:

1. write `bootstrap.py` (embedded default, or a test-injected override) into the
   runtime-python cache dir,
2. `bootstrap.spawn_stdio(...)` → piped, `kill_on_drop` child,
3. concurrently drain stdout (protocol reader) and stderr (capped log buffer),
4. `tokio::select!` across: handshake read (handshake timeout), terminal read
   (wall timeout), and `cancel.cancelled()`,
5. on terminal `result`/`error` → build `ScriptRunOutcome`; on
   timeout/cancel/protocol failure → kill + reap, return the matching error,
6. always await the child so no zombie remains (`kill_on_drop` is the backstop).

Output caps: stdout and stderr each bounded by `max_stdout_bytes` /
`max_stderr_bytes`; exceeding the cap truncates and surfaces
`OutputCapExceeded` (protocol stream) or a truncation flag (stderr log).

## Error handling

`HarnessScriptError` (thiserror) variants:

- `Spawn` — interpreter resolve/launch failed
- `HandshakeTimeout` — no `ready` within the handshake window
- `ProtocolMismatch` — `ready` version ≠ `PROTOCOL_VERSION`
- `MalformedMessage` — undecodable/unexpected frame
- `WallTimeout` — no terminal message within the wall-time cap
- `Cancelled` — caller cancelled via the `CancellationToken`
- `ScriptError { error_class, message, traceback }` — script raised
- `OutputCapExceeded` — protocol stdout exceeded its byte cap

Messages are structured enough for a future LLM to repair its script (an
acceptance-criteria requirement carried forward from the issue).

## Testing

Rust unit tests in `runner_tests.rs`, gated on a system Python
(`RuntimePythonConfig { prefer_system: true, .. }`); each test resolves the
interpreter first and **skips cleanly** if none is present, matching
`runtime_python/bootstrap_tests.rs`. A `with_bootstrap_source(String)` test seam
lets tests inject a pathological child to drive protocol/lifecycle failure paths
without any orchestration layer.

Cases:

1. **happy path** — script calls `set_result(inputs)`; outcome round-trips the JSON.
2. **script error** — script raises; outcome is `ScriptError` with class/message.
3. **wall timeout** — injected sleeper child; `WallTimeout`, child is killed.
4. **cancellation** — cancel a long-running child mid-run; `Cancelled`, killed.
5. **bad handshake** — injected child prints a wrong-version/garbage first line; `ProtocolMismatch`/`MalformedMessage`.
6. **output cap** — injected child floods stdout; `OutputCapExceeded`.

## Verification plan

Building the full desktop core requires native toolchains (CMake, Ninja,
Whisper, CEF) that may not be available in every environment. Verification is
best-effort: `cargo check` / targeted `cargo test harness_script` where the
crate compiles, with an honest report of what compiled and ran versus what was
blocked by native dependencies.

## Open questions (deferred to later phases)

Carried from the issue and intentionally out of Phase 1 scope: whether script
runs reuse `workflow_runs` storage or a distinct ledger; one-shot vs.
persistent per-run Python state; whether static DAGs eventually compile to
scripts; import allowlist policy; UI treatment of script artifacts.

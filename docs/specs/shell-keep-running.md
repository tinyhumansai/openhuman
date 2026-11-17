# Keep-running shell commands

**Status:** built (phase 1); phase 2 not started
**Scope:** `crates/openhuman-core/src/tools/impl/system/shell*.rs`,
`crates/openhuman-core/src/sandbox/shell_exec.rs`, a new per-turn process
registry, one new agent tool; TinyBox only for the optional phase 2
**Related:** tinyhumansai/openhuman#8226 (a timed-out command keeps its output),
tinyhumansai/tinybox#113 (a finished command returns even when a daemon holds
its pipes)

## Problem

The `shell` tool blocks until the command ends or its deadline kills it.

- **Killing loses output.** A test, fuzzer or build that is still working at
  its deadline is killed. #8226 returns what reached the tool before the kill,
  but output held inside the process (C stdio, Ruby and Perl buffers) or
  inside a pipeline (`… | tail -40`) is lost. Across 85 timed-out shell calls
  in our Terminal-Bench 2 runs, 62 were piped to `tail`, `head`, `sort` or
  `grep`. No environment switch or buffering change recovers those.
- **Killing loses the work.** A 40-minute test that is 30 minutes in is gone,
  and so is the time it took.
- **Blocking loses the turn.** While one call blocks, the model cannot look at
  what the command has printed so far, decide it is going the wrong way, and
  stop it. On `regex-chess` (c14) a self-test with no timeout blocked for 43
  minutes. Its first failure was printed in the first minute, and the model
  never saw it.

## How other harnesses handle it

| Harness | Long command |
|---|---|
| OpenHands | After a stretch with no new output ("no-change timeout") the call returns what was printed so far with exit code `-1` (still running). The model sends an empty command to keep waiting, or keys to interrupt. |
| Codex CLI (`unified_exec`) | `exec_command` returns after `yield_time_ms` (10 s by default) and the command keeps running; `write_stdin` polls it. |
| Terminus 2 (Terminal-Bench's agent) | A tmux terminal: keystrokes plus a wait, then the screen is read; a long command keeps running. |
| Claude Code | A hard timeout (2 min default, 10 max); for long work the model sets `run_in_background` and is notified on completion. |

None of these kills a long command at a fixed deadline by default. The cost
they report is runaway polling: a Codex session polled a finished command for
about three hours, and another spent about 55M tokens re-sending context on
polls (openai/codex #53659, #38495).

## What TinyBox already provides

Checked against the vendored TinyBox (`c2e34c5`):

- A native shell runs as a process the host reads by cursor
  (`read_output(process, cursor)` returns chunks and `next_sequence`). Reading
  does not consume anything, and the process keeps running whether or not the
  host reads (`tinybox-module/src/tinybus_module/output.rs`, `OutputJournal`).
- The process stops on its own runtime budget (`timeout_ms` in the request,
  up to 24 h; it ends as `Failed { TimedOut }`), on `cancel`, or when it
  exits. `is_running(process)` answers without reading.
- `release_output` is accepted only once cleanup is acknowledged, that is
  after the process has stopped; the host then closes the command's resource.
- **Limit:** the journal keeps every chunk until release, and the total is
  capped at `MAX_OUTPUT_BYTES` = 1 MiB per process. Past it, the process is
  stopped with `Failed { OutputLimit }`. A chatty command kept alive for many
  minutes can reach it.
- Up to 32 stream executions per module instance (`MAX_STREAM_EXECUTIONS`).

Today the host (`sandbox/shell_exec.rs`) reserves a resource, creates it,
starts the shell, reads in a loop until exit or its own deadline, cancels on
the deadline, releases the output and closes the resource, all inside one
tool call. A dropped wait (cancelled turn) cancels the command.

So the capability exists. What is missing is a host that lets the process
outlive one tool call.

## Design

### Tool surface

In a turn, `shell`'s `timeout_secs` is a **yield point**, not a kill.

- If the command is still running `timeout_secs` after it started (default
  120 s, at most 600 s; `0` waits for the end), the call returns **without
  cancelling**:

  ```
  Still running as process p3 after 2m 0s; it keeps running (it stops at its
  10m 0s timeout). Output so far:
  [stdout]
  …
  Follow it with shell_process {"process": "p3"} to wait for more output or
  the end, or {"process": "p3", "action": "kill"} to stop it. …
  ```

  The result is not an error. A command that finishes before the yield point
  returns exactly as today.
- The yield point comes no later than an eighth of the command's deadline
  (not earlier than 30 s for that cap) and always before the deadline itself,
  so a command started late in a turn still yields before the turn kills it.
- A deliberate wait, `sleep` followed only by commands that look at the
  result (`sleep 60; tail -20 build.log`), runs to its end instead of
  yielding with nothing to show. Claude Code makes a similar exception.
  `sleep 2; make -j8` yields as usual.
- **A call that cannot yield is still bounded by the model's time.** When the
  turn already follows four processes, when the command is a deliberate wait,
  or on a backend that cannot leave a command running (Docker), the model's
  `timeout_secs` stays the command's deadline, as before.
- **Only where the model can follow.** The host middleware moves
  `timeout_secs` aside only in a turn that registers `shell_process`. An
  agent whose tool belt names `shell` alone keeps `timeout_secs` as a kill
  and never yields.
- **The kill is the turn's.** TinyAgents' `ShellTurnBudget` clamp writes the
  turn's remaining budget into `timeout_secs`, and the shell passes that to
  TinyBox as the command's `timeout_ms`. So a host middleware
  (`agent/tinyagents/middleware/shell_yield.rs`), registered just before the
  clamp, moves the model's `timeout_secs` to the shell's internal yield
  argument (`yield_after_secs`); the clamp then sees a call that asked for no
  deadline and sets it as before. When the deadline fires, the result is the
  #8226 timeout report.
- For a kill of its own, the model writes coreutils `timeout N` into the
  command, as it already does.
- Outside a turn (CLI, JSON-RPC) nothing moves `timeout_secs`: it is the
  caller's deadline, and nothing yields.

Why one knob: on the Terminal-Bench baseline the model set `timeout_secs` on
16% of shell calls, mostly 300, 600 or 900 s, as an estimate of how long the
work takes, and wrote `timeout N` into 95 commands when it wanted a kill. Of
27 killed calls, 15 asked for no deadline and died at the turn's clamp. With
two knobs, a short `timeout_secs` silently meant "never yield, kill at N".

A new tool, `shell_process`:

| Argument | Meaning |
|---|---|
| `process` | The id a yield returned. |
| `action` | `wait` (default), `kill` or `status`. |
| `wait_secs` | For `wait`: how long to block before returning, default 120, clamped to 1–600. The process's own deadline still stops it. |

- `wait` blocks host-side until the process exits or `wait_secs` passes, then
  returns only the output written since the last read, plus "exited with
  code N" or "still running (Nm Ns)".
- `kill` cancels it and returns its remaining output.
- `status` says whether it is still running, without output. It refreshes
  the state with a read that returns at once and keeps that output for the
  next `wait`, so a process that has ended says so and frees its place.
- A `kill` that did not take within 10 s is reported as such; the process
  keeps its place until it stops.
- A `wait` on a process that has already exited and been read returns
  "already exited with code N" at once, so a loop cannot spin on it.

### Process registry (OpenHuman host)

A per-turn registry, e.g. `sandbox/shell_processes.rs`:

- An entry holds the live handle (TinyBox resource, process and read cursor),
  the start time, the command line (for display) and the hard deadline.
  Output read after the yield goes to the model in `shell_process` replies,
  not through the shell's progress meter.
- Ids are short and turn-local (`p1`, `p2`, …).
- At most 4 live processes per turn. A call reserves its place before the
  command starts, so parallel calls cannot overshoot; a command that finishes
  first frees it. A fifth command waits for its end, bounded by the model's
  `timeout_secs`.
- **Audit:** a command left running is audited when it ends (or is stopped
  at turn end), with its real outcome and duration, not as a success at the
  yield.
- When a process has stopped and its output has been read, the registry
  releases the output and closes the resource.
- **Turn end:** the registry lives on the turn's run context
  (`OpenHumanRunContext::shell_processes`; a child run gets its own). Dropping
  it at the end of the turn drops every handle, and a handle dropped while its
  process runs cancels it, waits up to 10 s for it to stop, releases its
  output and closes its resource in the background. Processes do not outlive
  the turn; long-lived services are started detached, as today (tinybox#113
  makes that return promptly).
- Direct dispatch (CLI, JSON-RPC: no run context) has no registry, so `shell`
  waits for the end there as before.

### Output

- Each reply carries the new output since the last read, under the existing
  tool-output caps. Longer output is kept by the harness and previewed, as for
  any tool result.
- The yield and `wait` replies label streams with `[stdout]` and `[stderr]`,
  like #8226.
- The 1 MiB journal cap is a hard stop. The reply names it ("TinyBox stopped
  the command: its output exceeded the execution output limit"). See phase 2.

### Model guidance

The shell tool's description says that a command still running after
`timeout_secs` comes back with its output so far and keeps running as a
process, to follow it with `shell_process` rather than start a second copy,
that it is stopped when the turn's time runs out, that `timeout_secs` is when
the call comes back rather than a kill (put `timeout N` in the command for
that), and not to pipe a long
command through `tail` or `head` (a pipeline holds its output until the
command ends, so a yield shows nothing).

### Failure classification and recovery

- A yield is not a failure; the repeat and no-progress guards do not count it.
- A `kill` by the model is not a failure.
- A process that reaches its hard deadline reports exactly as a timed-out
  command does today (#8226), and keeps the shell timeout class.
- The no-progress guard counts consecutive `wait` replies that carried no new
  output on the same process. From the third, the reply suggests stopping it,
  and the tool-loop limits still apply.

### Paths

| Path | Keep-running |
|---|---|
| Native TinyBox shell (default) | yes |
| Native jail | yes (same process API) |
| Docker module (`run_docker`) | no: it is one-shot and has no process handle; the model's `timeout_secs` is its deadline, as before |
| Sandbox-policy path (sandboxed agents, SaaS) | yes on the no-sandbox and native-jail backends; Docker as above |
| Cron jobs | unchanged; they keep blocking semantics |
| Windows | yes, nothing here is Unix-specific |

## Phase 2 (TinyBox, only if the 1 MiB cap bites)

Let the host trim what it has read: `release_output_until(process, cursor)`
drops chunks below the cursor, and the cap applies to bytes still held rather
than to everything the process ever wrote. A long-running chatty command then
runs to its deadline. This is a contract change in TinyBox, so it waits for
evidence that the cap is reached in practice.

## What this replaces

- **"Half the remaining turn for an unbounded command"** (held, not
  published): with a yield point, an unbounded command no longer blocks the
  turn, so it needs no special cap.
- **A pseudo-terminal for every command:** rejected as too invasive (pagers,
  colours, password prompts, merged streams). Keeping the process alive
  recovers buffered and piped output without it.
- **`stdbuf` preloading:** Linux-only and fragile; not needed once nothing is
  killed early.
- **A "don't pipe to `tail`" line in the description:** kept, since a
  pipeline shows nothing at a yield either.

## Risks

- **Polling loops.** Mitigated by host-side blocking waits, the
  already-exited answer, the no-new-output note and the turn's tool-loop
  limits. The Codex reports are the cases to test against.
- **Forgotten processes.** Bounded by the per-turn cap, the hard deadline and
  turn-end cleanup.
- **The model starts a duplicate.** The guidance line, plus the yield reply
  naming the running process.
- **Token cost.** Each wait re-sends the conversation; a long wait (120 s
  default) keeps that to a few calls per command.

## Tests

- Unit, through the test stand-in (`sandbox/shell_processes_tests.rs`,
  `tools/impl/system/shell_process_tests.rs`,
  `tools/impl/system/shell_tests_keep_running_tests.rs`):
  - a command finishing before the yield point returns as today;
  - a command past it yields with its output and is followed to its end;
  - `wait` returns only new output, then the exit code; a non-zero end reads
    like a failed `shell` call;
  - `kill` stops it and returns the rest;
  - asking about an ended process again says how it ended;
  - waits with nothing new are counted, and the reply suggests stopping it;
  - at most four processes are left running;
  - dropping the registry stops its processes;
  - the runtime budget still stops a process, as a timeout;
  - `timeout_secs: 0`, a command that starts with `sleep`, and direct
    dispatch wait for the end;
  - the yield point's default, ceiling and deadline-relative cap;
  - the turn moves the model's `timeout_secs` to the yield argument, and the
    turn-budget clamp then sets the deadline.
- Not covered by a unit test: the 1 MiB cap (the message comes from the
  existing TinyBox failure mapping).
- Live, on the released TinyBox through the Terminal-Bench rig (no automated
  test): processes survive between tool calls, are followed, killed and
  stopped at turn end (`mailman`, `gpt2-codegolf`, `regex-chess`).

## Work

1. OpenHuman (done): `run_yielding` and the live handle
   (`sandbox/shell_exec.rs`, `sandbox/shell_live.rs`), the registry
   (`sandbox/shell_processes.rs`), the `shell` yield, `shell_process`,
   turn-end cleanup, description, tests.
2. Bench validation: single-task runs so far; a full-suite A/B against the
   no-yield baseline is still to do.
3. TinyBox phase 2 only with evidence.

## Open questions

- The default yield point: 120 s total, or "no new output for 30 s" as in
  OpenHands? A total is predictable and doesn't cut short a computation that
  prints nothing; a silence rule returns sooner on a daemon that printed its
  startup and went quiet. Built: the total. A silence rule waits for an A/B.
- Other harnesses, for reference: Codex CLI yields after 10 s (0.25–30 s) and
  has no hard kill in interactive mode; Terminus 2 waits a model-chosen
  duration of at most 60 s; OpenHands returns after 30 s without new output;
  Claude Code moves a command to the background at its timeout (120 s
  default, 600 s max) instead of killing it.
- Should processes survive into the thread's next turn (a dev server started
  by one message, inspected by the next)? This spec says no: turn-scoped
  only.

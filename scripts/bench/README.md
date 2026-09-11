# `scripts/bench/` — agent-scale benchmarks

Drive a **real `openhuman-core` server process** at concurrency against a
mocked LLM, sample its CPU and memory from the outside, and report a leak
verdict.

## How this differs from `scripts/profile/`

Both measure resources; they answer different questions, and the difference is
the reason this directory exists rather than another scenario in the old one.

|                | `scripts/profile/`                      | `scripts/bench/` (here)                       |
| -------------- | --------------------------------------- | --------------------------------------------- |
| Core runs as   | a library, embedded in the bench binary | a normally-built `openhuman-core serve` process |
| Driven through | direct `Agent` calls                    | JSON-RPC over HTTP `/rpc`                      |
| LLM mocked by  | a native `ChatModel` override           | an HTTP endpoint the core dials               |
| Needs          | `--features rss-bench`                  | nothing — the shipped feature set             |
| Measures       | domain and harness cost in isolation    | what the OS charges the shipped binary        |
| Best for       | attributing cost to a subsystem         | leak hunting, capacity, tail latency          |

`scripts/profile/` cannot see transport, serde, connection handling or the
scheduler, because in that tier none of them run. This one includes all of it
but attributes less precisely. Use `profile/` to find out *what* costs; use
`bench/` to find out whether the thing you ship *grows*.

## Requirements

- Linux (the sampler reads `/proc`).
- Node 20+.
- **Artifacts on a disk-backed filesystem, not tmpfs.** The runner puts the
  core's workspace under `--out-dir` and refuses to start if that lands on
  tmpfs. This is not fussiness: tmpfs pages *are* memory, so the core's disk
  writes would be charged against the machine's RAM while the benchmark is
  trying to attribute RAM to the core — and a sustained run fills the mount,
  after which turns fail with "Failed to write auth profile lock owner" and
  SQLite I/O errors. That looks like a leak-induced meltdown and is a full disk.
- **Several GB free.** A 5-minute run at concurrency 8 leaves ~5 GB of memory
  chunks and embeddings behind.
- A release core binary:

```bash
cargo build --release --bin openhuman-core \
  --no-default-features --features "$(bash scripts/ci/product-features.sh)"
```

Release matters. A debug binary's allocation behaviour and CPU cost are not the
product's, so a leak verdict taken from one says little.

## Run it

```bash
scripts/bench/run-agent-scale.sh                      # defaults: 8 concurrent, 300 turns
scripts/bench/run-agent-scale.sh --concurrency 32 --turns 2000
scripts/bench/run-agent-scale.sh --duration-ms 900000 --tool-depth 3   # 15-minute soak
```

Exit status is the verdict: non-zero when a leak or drift check fails.
Artifacts land in `target/bench/<timestamp>/`:

| File             | Contents                                        |
| ---------------- | ----------------------------------------------- |
| `report.json`    | verdicts and the numbers behind them            |
| `samples.jsonl`  | the raw resource series                         |
| `driver.json`    | throughput, latency percentiles, error buckets  |
| `turns.jsonl`    | per-turn latency and outcome                    |
| `mock-stats.json`| what the mock actually served                   |
| `core.log`       | core stderr                                     |

Runner options (the script's `--help`):

```text
--concurrency N     parallel in-flight turns (default 8)
--turns N           total turns in the measured window (default 300)
--duration-ms N     run for a wall-clock duration instead of a turn count
--warmup-turns N    turns to run and discard before measuring (default 10)
--tool-depth N      tool calls the mock drives per turn (default 1)
--latency-ms N      mock inference latency (default 40)
--jitter-ms N       jitter around that latency (default 20)
--reply-chars N     assistant reply size (default 240)
--fail-rate F       fraction of completions answered 500 (default 0)
--thread-mode M     fresh | per-worker | shared (default fresh)
--interval-ms N     resource sampling interval (default 250)
--tree              also sample descendant processes
--keep-workspace    do not delete the temp workspace on exit
--workspace DIR     reuse an existing populated workspace (implies --keep-workspace)
--memory-off        disable memory reads and writes (recall + learning)
--memory-writes-off disable memory writes only, keep recall reads (mutually
                    exclusive with --memory-off)
--out-dir DIR       where to write artifacts (default target/bench/<stamp>)
```

### Memory comparison controls

- `--memory-off` disables recall, autosave, learning hooks, embeddings, and the
  memory tree. Compare it with the default run to separate memory cost from
  unrelated uptime or harness effects.
- `--memory-writes-off` disables autosave and learning writes but keeps recall.
  Use it with `--workspace DIR` pointing at populated data to distinguish read
  cost from write cost.
- `--workspace DIR` reuses caller-owned data and keeps it after the run. Pair
  consecutive runs against the same populated corpus to distinguish data-size
  effects from process-uptime effects.

The two memory-disable flags are mutually exclusive: `--memory-off` already
includes the write suppression performed by `--memory-writes-off`.

## How it works without touching the core

Two facts carry the whole design, and a change to either breaks this tier:

1. **`BACKEND_URL` redirects inference.** The core derives both its inference
   base and its backend base from that one value, so pointing it at the mock
   captures chat completions, embeddings and Langfuse telemetry together.
2. **A session token shaped `<a>.<b>.local` skips backend validation.** Storing
   one persists a profile without the `GET /auth/me` round-trip a remote JWT
   would trigger, so the benchmark needs no login and the mock needs no auth
   routes. The driver seeds it before the load starts.

The mock also must not listen on 11434, 8000, 8080, 1234 or 8888 — the core
classifies those as local-AI endpoints and routes around them. `mock-llm.mjs`
refuses to start on one rather than failing mysteriously later.

Three further details the runner handles, each of which silently ruins a run if
you reproduce this setup by hand:

- **The approval gate must be off** (`OPENHUMAN_APPROVAL_GATE=0`). It is on by
  default and parks interactive chat turns pending a human decision, with a
  10-minute TTL that resolves to Deny.
- **The daily cost limit must be raised.** The core prices the mock's reported
  token usage against a $10/day managed-inference budget and a sustained run
  exhausts it in a few hundred turns, after which every turn fails instantly.
  The runner raises the limit rather than disabling the check, so the budget
  check's own cost stays in the measurement.
- **The embedding width must match** (the mock serves `--embed-dims`, default
  1024; the runner never overrides it, so only hand-launched mocks can diverge).
  A mismatch is only a warning: chunks are stored without vectors and the memory
  write path runs degraded for the entire run.

## Baseline

From a 5-minute run at concurrency 8, tool-depth 1, on one machine — indicative,
not a target, and not yet reproduced across hosts:

| Measure                | Value                          |
| ---------------------- | ------------------------------ |
| Turns                  | ~8,500 (0 failed)              |
| Throughput             | ~29 turns/s mean               |
| Latency                | p50 260 ms, p99 570 ms         |
| CPU                    | ~6.9 cores mean, ~245 ms/turn  |
| RSS                    | 1.48 → 2.33 GiB                |
| Threads / FDs          | stable                         |
| Workspace written      | ~5 GB                          |

Two findings from that run reproduced across repeats and are worth chasing
rather than treating as harness noise: RSS grew ~115 KiB/turn and was **still
growing at the end** (confounded by the 5 GB of workspace growth — needs the
memory-disabled comparison to settle), and **throughput fell to ~37% of its
starting rate** under constant offered load, which the workspace growth does not
obviously explain.

## Reading a verdict

Memory gets three outcomes, and the middle one is the point:

- **pass** — no growth trend, or growth within the per-turn budget.
- **plateau** — grew past the budget overall, but stopped climbing by the final
  third of the window. The shape of a cache filling to its working set. Worth
  re-running longer to confirm the plateau holds.
- **fail** — grew past the budget *and was still growing at the end*.

That distinction is why the analyzer fits the tail of the series separately
rather than comparing an early average to a late one. Early-vs-late cannot tell
"grew then stopped" from "never grew", and treating a saturating cache as a
leak trains people to ignore the check.

**Threads and open file descriptors are held to a stricter standard.** Neither
has a legitimate reason to climb without bound under steady load, so they are
straight thresholds rather than trend tests, and they fail independently of
memory. In practice they are the least ambiguous leak signal available.

**Memory verdicts can be `confounded`.** `fresh` thread mode stops conversation
history accumulating, but it does not stop the agent persisting memory chunks
and embeddings every turn — a 5-minute run writes gigabytes. An index over data
that genuinely grew is not a leak. So when RSS fails alongside large workspace
growth, the report marks the verdict `confounded` and says what it cannot rule
out, rather than asserting a leak it cannot distinguish from correct behaviour.
To separate the two: re-run with `--memory-off` (memory capture disabled), or
run long enough that on-disk growth levels off while RSS keeps climbing.

**Throughput held / liveness.** The analyzer checks that turns kept completing,
because on resource metrics alone a dead process is indistinguishable from a
healthy idle one — flat memory, no CPU, stable threads. An early version of this
report gave a confident PASS on a run where the core had stopped answering two
thirds of the way in. A total outage additionally marks `livenessBroken`, which
qualifies every other verdict; mere degradation does not, because the process
was still working and its resource numbers remain real.

**CPU drift** compares CPU consumed per unit wall time between the start and
end of the window. Under constant offered load a rising figure means each turn
is costing more than the last — the CPU analogue of a memory leak, typically an
unbounded structure being rescanned every turn.

### Thread mode decides what you can conclude

```bash
--thread-mode fresh        # default: a new thread per turn
--thread-mode per-worker   # one long conversation per worker
--thread-mode shared       # all workers on one thread
```

Only `fresh` supports a leak verdict. In the other two, conversation history
accumulates *by design*, so RSS growth is expected and a leak is
indistinguishable from correct behaviour — the analyzer reports the growth rate
and explicitly declines to judge it. Use them for contention and tail latency,
not for leak hunting.

## Tuning the mock

```bash
--tool-depth N     tool calls per turn before the final answer (exercises the
                   agent loop, not just a single completion)
--latency-ms N     mean inference latency; realistic values keep many turns
--jitter-ms N      in flight and change the concurrency profile entirely
--reply-chars N    reply size — varies serde and allocation pressure
--fail-rate F      fraction of completions answered 500, to exercise retries
```

`--tool-depth 0` measures the RPC and inference path alone. Anything above zero
puts the agent's tool loop under test, which is where per-turn state actually
accumulates — so a leak hunt should use at least 1.

## Tests

```bash
node --test scripts/bench/analyze.test.mjs
```

The analyzer's failure mode is silence: wrong math reports "pass" on a leaking
run and nobody notices. The tests drive it with synthetic series whose correct
verdict is known by construction — a steady leak, a plateau, a flat line,
thread and FD growth, CPU drift — so a regression in the leak math fails loudly.

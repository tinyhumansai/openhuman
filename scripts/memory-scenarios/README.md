# Memory scenarios

Realistic memory flows against the real OpenHuman core, on two engines, checked
by the script itself. Every failed check becomes a finding for a person to
read; **nothing is fixed during a run**.

```bash
# both engines (local first, then Built-in on your real account)
node scripts/memory-scenarios/run.mjs

# one engine, some scenarios
node scripts/memory-scenarios/run.mjs --engine local --only A,B,E

# keep the local CortexDB volume for a look afterwards
node scripts/memory-scenarios/run.mjs --engine local --keep

# re-render the report of an old run
node scripts/memory-scenarios/run.mjs --grade-only target/memory-scenarios/<run-id>
```

The driver is [`run.mjs`](./run.mjs), with scenarios in [`scenarios.mjs`](./scenarios.mjs) and the two engine setups in [`engines.mjs`](./engines.mjs). Requires a core build (`cargo build -p openhuman-cli --bin openhuman-core`),
Docker, and for the local
engine's embeddings a running Ollama with `nomic-embed-text` (and `llama3.2:3b`
for CortexDB's extraction and answer lanes; both pulled when missing).

## The account credential: a file, passed through the environment only

Chat runs on the account's managed route on both engines, and the builtin engine
stores memory on the account, so the run needs a credential:

| file (chmod 600)                                 | what                                                                                           | reaches the core as               |
| ------------------------------------------------ | ---------------------------------------------------------------------------------------------- | --------------------------------- |
| `~/.memscen-session` (or `MEMSCEN_SESSION_FILE`) | the app's session JWT, preferred for builtin                                                   | `OPENHUMAN_BACKEND_SESSION_TOKEN` |
| `~/.memscen-key` (or `MEMSCEN_KEY_FILE`)         | a TinyHumans API key (`tiny_…`); the local engine's chat, and builtin when there is no session | `OPENHUMAN_BACKEND_API_KEY`       |

The script reads the file when it spawns the core and hands the value over in
the core's environment only; the core seeds it at boot
([`security/credentials/ops/boot_env.rs`](../../crates/openhuman-core/src/security/credentials/ops/boot_env.rs)). It is never an RPC argument, never
printed and never written to the run directory: `core.log`, `rpc.jsonl` and
every error text are scrubbed of the value itself and of anything `tiny_…` or
JWT-shaped (`eyJ…`). A core spawned for an engine that should not have a
credential gets neither variable, even if your shell exports one. Revoke the
session on the backend when you are done. An API key needs the **`memory`**
scope for builtin; without it every memory call answers 403.

## Headless, but shaped like the desktop app

Same rig as [`life-scenarios`](https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/life-scenarios/README.md) in openhuman-benchmarks: the core runs
as `openhuman-core serve` with **its own `HOME`** under
`target/memory-scenarios/<run-id>/<engine>/home`, and turns go through
`openhuman.channel_web_chat` with the reply on `GET /events`, exactly like the
composer. Memory RPCs (`memory_learn`, `memory_items_list`, `memory_recall`,
`memory_brain_ingest`, `memory_sources_*`, `memory_import_*`,
`memory_migration_*`) are called the way the Memory page calls them.

## The two engines

**local**: a throwaway CortexDB **v0.10.5** per run ([`cortexdb/`](./cortexdb) here) with its
own compose project `memscen-<run>`, its own volume and a free port. It never
touches the user's `cortexdb` container, its `cortexdb-data` volume or port 3141,
and is torn down with its volume at the end, or on Ctrl-C (`--keep` keeps the
volume). Its embeddings are the local Ollama's `nomic-embed-text` (768 dims,
through Ollama's native API at the bare base URL: under `/v1` CortexDB's
`ollama:` provider 404s and every store waits out a 30 s index timeout). When
Ollama is unreachable it falls back to tinymemory's deterministic mock
inference, and the report says the recall-quality checks were skipped.

- Chat runs on the account's managed route through the API key; **memory stays
  on the container**. The run aborts unless `memory_engine_get` reports
  `cortexdb` at the run's own endpoint, before and after setup. The scheduler
  gate is off on this core too (written to the active config and read back),
  so no background job runs while it holds the key. If the managed route
  cannot run, chat falls back to the local Ollama and the checks that depend
  on reply quality are recorded **INCONCLUSIVE**, not failed.
- The core talks to CortexDB through a logging pass-through, and every request
  body lands in `local/cortex-wire.jsonl` (scrubbed). Attribution
  (`observed_actor`, `subject`) is written on the wire and not returned on
  read-back, so those checks read the wire.
- **E (the move into layout v3) runs on a fresh core that never saw the key**:
  a user-started move is gated on background work, which the key-holding core
  keeps off, and this core may run background work without any way to reach
  the account.

**builtin**: **your real account**, on its managed route. Read this before
running it.

- **Nothing of the account's own is moved, erased, imported or consolidated.**
  Guards; any failure aborts the builtin run before a scenario starts:
  1. the scheduler gate is pre-written **off** in every config the core may
     treat as active (the root, the pre-login `users/local/`, and for a session
     the account's own `users/<subject>/`), and read back from the config the
     core actually activated, so no background job (the layout migration's
     tick, import resume, belief builds) runs on its own;
  2. **Composio is off** (`[composio] mode = "disabled"`, read back the same
     way), so no turn can reach the account's connected mail, calendar or
     repositories;
  3. the active workspace's layout-migration state is written as done
     (`cleaned`) and read back through `memory_migration_status`;
  4. no v1 store exists in the throwaway workspace.

  A builtin turn may call only memory-side tools (`memory`, `resolve_time`,
  the harness's own bookkeeping). The shell tool has no off switch, so this one
  detects rather than prevents: the first turn that calls anything else stops
  every remaining scenario and is reported as a finding.

  Migration, import, erase, disconnect-with-clear and belief-build scenarios
  run **local only**. At the end the run asserts the migration state never
  moved and no import ran.

- **Everything the run writes is removed.** Each item carries a run marker
  (`thread-<uuid>` threads, a `memscen:<run>` tag); every id it stores is
  recorded; teardown forgets exactly those, plus every item in the run's
  threads and every item with its tag, then checks each id is gone and that
  the tag and every thread list empty. Survivors are
  reported as a finding. Beliefs the engine derives on its own from the run's
  items cannot be traced back to the run and are not cleaned up.
- **An empty answer is not taken as clean.** Before the real clean-up, a
  positive control stores a probe under the run's tag, waits to see it
  listed, forgets it (`forgotten: 1`) and waits to see it gone. A backend that
  answers "empty" while degraded fails the control, and the clean-up with it.
- **The clean-up fails closed.** The ids, threads and marker are written to
  `<run-dir>/builtin/cleanup.json` before anything is forgotten. Any error
  listing, forgetting or reading back fails the clean-up (it never reads as
  "nothing survived") and prints the retry, which boots the same guarded core,
  runs no scenario and forgets what the file names:

  ```bash
  node scripts/memory-scenarios/run.mjs --cleanup target/memory-scenarios/<run-id>
  ```

## Scenarios

The definitions live in [`scenarios.mjs`](./scenarios.mjs); the memory domain they exercise is documented in the [`memory` README](../../crates/openhuman-core/src/memory/README.md).

| id            | engines | what it checks                                                                                                                                                                                                                                       |
| ------------- | ------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A-storing     | both    | chats with two agents logged under each agent; learnings of every kind, kind visible on read-back; brain text; a file from a path holding a username (only the basename may reach the engine); dated facts                                           |
| B-recall      | both    | cross-thread recall; a negative control (no invention); a planted learning changes a reply; brain retrieval with citations; pre-turn pack timeouts at the default wait vs five seconds more (`[memory:hooks] pre_turn timed out` in core.log)        |
| C-dates       | both    | the user time zone; "yesterday" and a named date find the right fact; a Spanish question gets a Spanish reply                                                                                                                                        |
| E-migration   | local   | legacy items plus a v1 store: import with consent; a shared (self-hosted) tree waits for the takeover consent, then moves; a core restart mid-move; layout v3; nothing lost; v1 documents present; chats pooled at `ws:main`; the same recall answer |
| F-labels      | local   | thread and agent on every logged turn; phone numbers in plain text; on the wire, `observed_actor`/`subject` on assistant turns only with `[memory] observed_actor = true`, none with it off                                                          |
| G-source-cap  | both    | more than four brain sources: at most four in the pack, the named one included, no Team section                                                                                                                                                      |
| H-forget      | both    | a forget removes from list and recall; forgetting nothing reports `forgotten: 0`                                                                                                                                                                     |
| I-qa          | both    | belief build (`memory_jobs_run`, local only), backfill state, a new learning listed at once; local: the engine stopped mid-session, is the failure surfaced or swallowed                                                                             |
| J-store-speed | builtin | a synthetic batch stored on the hosted path, timed, with `UNAVAILABLE`/429 counted (no real import)                                                                                                                                                  |

Not driven here (said so in the report): a workflow writing memory, workflow
memory kept out of chat, and the unconfirmed-import no-erase guard (unit-tested
in the core).

**Known gap, not a finding:** channel messages are not logged to memory today,
so the channel sender name (#7131) is inert.

## The corpus is fictional

[`fixtures/persona.json`](./fixtures/persona.json): one persona (Jordan Lee `<jordan.lee@example.com>`),
`*.example` domains, `555-01xx` phone numbers. The mock's mail and issues are
invented too. Dates the checks reason about are the real clock's, in the
persona's time zone, because the engine stamps items with the real time.

## Output

```
target/memory-scenarios/<run-id>/
  report.md          per engine: inference, scenario table, findings, notes
  findings.json      [{id, engine, scenario, severity, expected, actual, evidence, basis: READ|INFERRED}]
  checks.json        every check, passed or not
  results.json       measurements (pre-turn timeouts, store speed, migration counts, ...)
  <engine>/core.log  the core's log (scrubbed)
  <engine>/rpc.jsonl every RPC with its result (scrubbed)
  <engine>/scenarios/<id>/transcript.jsonl   every turn: message, reply, tools, time
  local/cortex-wire.jsonl                    every request the core sent CortexDB (scrubbed)
```

A finding's `basis` is **READ** when it was seen (a response, a log line) and
**INFERRED** when it was judged (a reply's wording, a heuristic).

## See also

- [Memory](../../gitbooks/features/memory.md) and [Memory architecture](../../gitbooks/developing/architecture/memory.md), the behavior under test.
- [`vendor/tinymemory/`](../../vendor/tinymemory) and its repo, [tinyhumansai/tinymemory](https://github.com/tinyhumansai/tinymemory).
- [`docs/specs/memory-v2.md`](../../docs/specs/memory-v2.md) for the layout spec.
- [`life-scenarios/`](https://github.com/tinyhumansai/openhuman-benchmarks/blob/main/life-scenarios/README.md) in openhuman-benchmarks, and [`scripts/README.md`](../README.md).

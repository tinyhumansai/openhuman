# memory

This is the host side of agent memory. TinyMemory (`vendor/tinymemory`) owns
the engine contract, the turn lifecycle and the engines themselves; this
folder binds one engine to OpenHuman, works out who is acting, calls the
lifecycle around every agent turn, queues the slow work, and exposes it all
through the `openhuman.memory_*` controllers, the `memory` agent tool, cron
jobs and the event bus. OpenHuman policy (credentials, consent, scrubbing,
scheduling, deletion) is applied here. The design spec is
[`docs/specs/memory-v2.md`](../../../../docs/specs/memory-v2.md).

Chat thread persistence is not memory. It lives in
[`threads/store`](../threads/store/README.md).

## How it works

### Binding an engine

Every memory operation starts with `engine::resolve(config)`, which turns
the `[memory]` config into either `Binding::On(BoundEngine)` or
`Binding::Off` with a reason. Two engines are supported:

| Engine | Endpoint | Credential | Off when |
| --- | --- | --- | --- |
| `tinyhumans` | `[memory.engines.tinyhumans] endpoint`, else `backend::base_url` | the host's backend credential, resolved per request | signed out, or no backend transport installed |
| `cortexdb` | `[memory.engines.cortexdb] endpoint`, else CortexDB's managed API | the API key stored in the keyring as `memory-cortexdb` | no key stored |

The core never keeps a TinyHumans credential of its own. The bearer is
fetched from the host's credential seam (`resolve_backend_credential`) on
every request, so a refreshed session takes effect at once and signing out
turns memory off. The hosted engine does not go through the core's
`BackendTransport`: [`engine.rs`](./engine.rs) hands TinyMemory's CortexDB client an
`EngineSettings` with the endpoint, the attribution headers from
`backend::attribution_headers` and an `EngineCredential::Dynamic(HostBearer)`,
and TinyMemory's own HTTP client calls the backend's `/memory/*` routes. The
`cortexdb` engine calls CortexDB directly with the stored key
(`EngineCredential::Static`). Built engines are cached per config fingerprint (engine
id, endpoint, credential identity, layout), so a config change, a sign-in or
a new key rebuilds the engine on the next call without any event plumbing.

An embedding host can install its own engine with `install_host_engine`.
It wins over the configured one for every config in the process, so a host
with its own store (or a test binary with an in-memory engine) runs the
whole lifecycle without a TinyHumans session or a CortexDB key.

Whatever engine is bound, it is wrapped in `guard::ScrubbingEngine`. Every
text a `StoreItem` carries is scrubbed of secrets and PII under
`security::scrub::host_policy` before the engine sees it. Wrapping the
engine, instead of scrubbing at call sites, also covers the writes
TinyMemory makes through its own types (`AgentMemory::pre_turn`,
`Brain::ingest`, background ingests). The same wrapper logs one
`[memory:timing]` debug line per engine call with the op, engine, elapsed
time and outcome, never user content.

When memory is off, the `memory` tool is not registered, the turn hooks do
nothing, and the RPCs answer with the `MEMORY_OFF` error code.

### Who is acting

Memory follows TinyMemory's standard layout (`tinymemory_tools::MemoryLayout`):

```text
<root>                      shared learnings; holistic recall reads it all
 +-- source:<kind>          the brain: synced documents, no agent id
 +-- agent:<memory agent>   one agent's conversations, one turn per item
```

A `MemoryIdentity` (in [`scope.rs`](./scope.rs)) is the acting agent definition and, for a
team member, its team. The host scopes one around every agent turn with
`scope::within_agent` or `scope::within`, which set a task-local.
`MemoryIdentity::resolve` turns it into a `ResolvedIdentity` (layout root plus
memory agent id), first match wins:

| Rule | Memory agent id | Layout root |
| --- | --- | --- |
| 1. host binding | `[memory] agent_id` | `[memory] root` |
| 2. definition pin | `[memory.agents.<definition>] agent_id` | `[memory.agents.<definition>] root` |
| 3. team member | the definition id | `team:<team>` |
| 4. default | the definition id, else `assistant` | the default root |

The host binding is how a coordinating host (an embedder through
`openhuman_embed::AgentSpec::memory`) gives each agent it runs a memory of
its own: it derives a per-agent config with those two keys set. The
identity is never taken from model arguments.

Under `[memory] layout = "v3"` everything sits below the signed-in person's
own scope root (`org:<id>`, `scope::user_root`), and every agent's chats
share one chat node (`ws:main`, `scope::chat_node`), with the agent id kept
on each turn. The person's actor is `user:<id>` (`scope::actor_of_root`),
which is also the retired root still read during the move while
`[memory] legacy_user_segment_read` is on. A signed-out session gets its
root from a random install id minted once and recorded in
`<workspace>/memory/local_root.json` as `user:local-<id>` ([`local_root.rs`](./local_root.rs)),
used as `org:local-<id>`. If that file cannot be written or read, the
session has no root and memory stays off.

### Around every turn

TinyMemory's `AgentMemory` runs the lifecycle. [`lifecycle/mod.rs`](./lifecycle/mod.rs) builds one
for an identity under the `[memory.recall]` policy. The live turn calls
[`lifecycle/prefetch.rs`](./lifecycle/prefetch.rs), which reads completed context
immediately and queues engine work. [`lifecycle/hooks.rs`](./lifecycle/hooks.rs)
contains the underlying lifecycle operations and bounded manual wrappers:

```text
user message
   |
   v
session host prelude ----> prefetch::pre_turn
(agent/session_host/          takes a completed cached TurnPack (or none)
 runtime_session/             queues user log (index 2n), recall and
 memory_ingest.rs)            optional date hint in a scoped worker
   v
MemoryPackMiddleware ----> adds the pack to each model request of the
(agent/tinyagents/            turn as an ephemeral instruction; it is
 middleware/memory_pack.rs)   never written to the transcript
   |
   v
model + tools run, turn is committed
   |
   v
session host ------------> scoped background hooks::post_turn
                              logs the reply with a tool-call summary
                              (index 2n + 1); queues belief builds
   |
   v (later, when the transcript is compacted)
memory_summarizer -------> prefetch::compaction
(agent/tinyagents/            enriches from completed context only;
 memory_summarizer.rs)        queues recall of the folded-away turns
```

Turn indices follow the committed transcript: the user message of the turn
after `n` committed turns is `2n` and its reply `2n + 1`. They are stable
across retries and restarts, which is what lets TinyMemory treat a retried
turn as a replay. Automatic hooks never await engine I/O on the live turn
or the authoritative compaction summarizer. Cold or expired caches supply
no pack; completed packs are labelled as context from an earlier lookup,
which may be incomplete or unrelated to the current question. Explicit
recall/fetch is the path to a current answer. If completed recall refuses
the account (for example `INSUFFICIENT_CREDITS`), its pack is a notice that
memory is unavailable, rather than evidence that nothing is stored.

The cache is scoped to CoreContext, workspace, config fingerprint, resolved
memory identity and thread. Entries expire after 60 seconds. There are at
most 64 entries per context, 64 KiB per pack (including citations), eight
active workers globally and 16 pending operations per entry. Each operation
has a 120-second background deadline. When capacity is exhausted, automatic
work is skipped without making the turn wait. The cache itself stays in process memory.
Content invalidation clears packs and rejects stale completions while
preserving queued logs; engine/credential invalidation also clears pending
refreshes. Scoped spawns retain the caller's core and memory identity.

The explicit/manual `hooks::pre_turn` and `hooks::compaction` retain their
configured `pre_turn_timeout_ms` and `compaction_timeout_ms` deadlines; they
are not what the live prelude and compaction wrapper await.

A few details hang off the turn. For a message that arrived on a channel,
`lifecycle::sender::channel_actor` records the sender as the observed actor
(`user:+1555...` for phone-addressed channels, `user:<email>`, otherwise
`<channel>:<sender>`). The session host records the channel/thread pairing
in [`channels.rs`](./channels.rs) so a channel's memory can be forgotten later. With
`[memory.recall] date_hint` on, `lifecycle::date_hint::extract` makes one
small model call to work out which days the user means; a late or bad
answer just leaves the pack in its normal order. The pack's citations are
parked per thread (`tools::record_pack_citations`) and `web_chat` takes them
for the chat's memory chips (`tools::take_turn_citations`).

### Background work

`post_turn` and brain ingests return TinyMemory `BackgroundJob` values
(belief builds, deferred ingests) instead of running them inline.
[`lifecycle/jobs.rs`](./lifecycle/jobs.rs) queues them in `<workspace>/memory/jobs.json`,
de-duplicates them (two builds of one scope are one build), and runs each
once it is at least `[memory.recall] build_delay_secs` old, because a belief
build reads facts the engine extracts from the writes and that extraction
lags. An engine that cannot consolidate answers `Skipped`; the hosted engine
builds on its own schedule (`Scheduled`).

Memory registers two system cron jobs, handled by [`bus.rs`](./bus.rs):

```text
CronSystemJobDue
   |
   +-- memory_sources_sync --> sources::sync_due (start due source syncs)
   |
   +-- memory_background (every 5 min)
         1. import::resume_interrupted     (v1 import the app quit mid-way)
         2. layout_migration::tick         (move legacy memory, when free)
         3. lifecycle::jobs::run_due       (belief builds, deferred ingests)
         4. deletion::drain                (deletions still owed)
```

### The brain and sources

The brain is the set of documents every agent under a root shares, filed at
`source:<connector>` nodes with no agent id ([`brain.rs`](./brain.rs)). A file ingested
from the UI (`memory_brain_ingest`) goes under `files`; synced sources are
filed by what they read. Files are converted through [`convert.rs`](./convert.rs): PDF,
DOCX, PPTX and XLSX are streamed to the configured TinyDocs module when the
`documents` feature is on, then `NativeConverter` handles text, markdown, HTML
and code. The host carries only the bus contract and current config; it has no
local office parser fallback. Every ingest queues a belief build of the
source's scope.

Sources ([`sources/`](./sources/)) are the `[[memory.sources]]` registry in `config.toml`:
a folder, a file, a web page, a GitHub repository or an RSS feed, plus how
often to sync it (at least every 15 minutes). Sync reads through
`tinymemory-integrations`' readers and stores each item as a `Document`
whose `meta.source` is `{kind, id: <source id>}`, so removing a source can
forget exactly its items. Per-item failures are logged and skipped. Runtime
sync state (last run, status, item count) lives in
`<workspace>/memory/sources_state.json`, not in config.

### Deletion

Deleting a chat thread, forgetting a channel (one thread deletion per thread
it owned) or removing a source with `forget_items` asks the engine to delete
for good: matched items are forgotten by `memory_ids` with an explicit
`redact_events` cascade (the engine's default keeps events). When memory is
off, or the call fails, [`deletion.rs`](./deletion.rs) records a `PendingDeletion` (`Thread`
or `Source`) in `<workspace>/memory/pending_deletions.json`. The queue is
drained on the next stored credential (the `memory::pending_deletions`
subscriber on `CredentialChanged`, any kind but `cleared`) and on every
background tick, until it succeeds. `memory_erase_all` erases everything in
scope.

### Consent-gated uploads

Three flows upload data the user already has locally, and each refuses to
start without explicit consent and resumes after a restart:

- [`backfill.rs`](./backfill.rs) stores chats from before turns were logged, walking the
  thread store and writing the same turn items the live hooks would, tagged
  `backfill`. It stops short of the first turn live logging already stored.
  Progress is in `conversations_backfill.json`.
- [`import.rs`](./import.rs) (with [`import_retry.rs`](./import_retry.rs)) imports a v1 store from
  `<workspace>/memory/memory.db` through `tinymemory-import`. A blocking
  reader walks the store, the async side scrubs, stores and checkpoints
  every 25 items in `import_state.json`. Transient engine failures retry
  with backoff; a stop caused by something that will not pass on its own
  (credits exhausted, signed out) waits for the user.
- [`layout_migration/`](./layout_migration/) moves memory written before layout v3 from the legacy
  `app:tinymemory/...` tree into the per-user tree. It copies page by page,
  verifies each item by reading it back, switches reads and writes only
  after everything copyable is verified, catches up on writes made in
  between, then cleans up the legacy copies. State is in
  `layout_migration.json`. A legacy tree other accounts may share (a
  self-hosted CortexDB) moves only with the user's consent, claimed through
  a marker in the shared app directory (`claim.rs`).

Automatic runs of the import and the migration start only when
`billing::free_period_active` says the work costs the user nothing: always
for a non-hosted engine, and for the hosted engine only while the backend's
memory free period is on (`GET /memory/free-period`, cached for 60 seconds).
Anything unknown counts as not free.

### The RPC surface is confined

The `memory_*` RPCs (and the MCP memory tools that dispatch through them)
go through [`confine.rs`](./confine.rs). It resolves the identity in scope (the agent of a
running turn, else the config's own root identity), fills an unset `reach`
with that identity's subtree, refuses a wider reach with `INVALID_REQUEST`,
and places a `learn` with no namespace at the identity's learnings node.
The agent `memory` tool applies the same confinement and overwrites any
`reach` in the model's filter.

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](./mod.rs) | Module declarations and re-exports (`memory_is_on`, `MemoryTool`, the controller aggregators, `register_memory_subscribers`). |
| [`engine.rs`](./engine.rs) | `resolve` binds the configured engine or says why memory is off; per-fingerprint cache; host engine install; CortexDB key storage. |
| [`guard.rs`](./guard.rs) | `ScrubbingEngine`: scrubs every write and times every engine call. |
| [`scope.rs`](./scope.rs) | `MemoryIdentity`, `ResolvedIdentity`, resolution order, `within` / `within_agent`, layout v3 roots. |
| [`local_root.rs`](./local_root.rs) | The persistent install-id root of a signed-out session. |
| [`lifecycle/mod.rs`](./lifecycle/mod.rs) | Builds `AgentMemory` for an identity under the `[memory.recall]` policy. |
| [`lifecycle/hooks.rs`](./lifecycle/hooks.rs) | Lifecycle operations and bounded manual wrappers; `TurnPack`, `MemoryTurn`. |
| [`lifecycle/prefetch.rs`](./lifecycle/prefetch.rs) | Scoped in-memory packs and bounded automatic background refresh. |
| [`lifecycle/jobs.rs`](./lifecycle/jobs.rs) | The persisted background job queue and the `memory_background` run. |
| [`lifecycle/views.rs`](./lifecycle/views.rs) | Policy get/set, pack preview, agents list, jobs list/run for the UI. |
| [`lifecycle/sender.rs`](./lifecycle/sender.rs) | Maps a channel message's sender to an observed actor. |
| [`lifecycle/date_hint.rs`](./lifecycle/date_hint.rs) | Optional model call that works out which days a turn refers to. |
| [`ops.rs`](./ops.rs) | Engine list/get/set, recall, fetch, learn, forget, erase all, items list. |
| [`tool_budget.rs`](./tool_budget.rs), [`tool_writes.rs`](./tool_writes.rs) | Agent call deadlines and durable, owner-scoped admission/delivery of optional writes. |
| [`confine.rs`](./confine.rs) | Confines the RPC surface to the acting identity's subtree. |
| [`explore.rs`](./explore.rs) | The facet explorer behind `memory_explore` and `memory_items_get`. |
| [`brain.rs`](./brain.rs) | Brain source mapping and the `memory_brain_*` ops. |
| [`convert.rs`](./convert.rs) | The document converter chain used by brain ingest and file sources. |
| [`sources/`](./sources/) | The `[[memory.sources]]` registry (`mod.rs`), sync state (`state.rs`) and sync (`sync.rs`). |
| [`channels.rs`](./channels.rs) | Channel to thread records, for forgetting a channel. |
| [`deletion.rs`](./deletion.rs) | Hard deletes and the pending-deletion queue with its drain. |
| [`backfill.rs`](./backfill.rs) | Consent-gated, resumable storing of past chats. |
| [`import.rs`](./import.rs), [`import_retry.rs`](./import_retry.rs) | Consent-gated, resumable v1 import, and retrying refused items. Skips what v1 synced from Composio connectors (`import_open.rs::open_legacy` turns on `LegacyWorkspace::skip_connector_syncs`); the connectors re-sync it. |
| [`layout_migration/`](./layout_migration/) | Legacy tree to per-user tree migration: `job.rs` (end to end), `copy.rs`, `map.rs` (placement), `cleanup.rs`, `claim.rs`, `state.rs`, `service.rs` (RPC/tick entry points), `app_host.rs` (the real host). |
| [`billing.rs`](./billing.rs) | Whether background rewrites are free for the user right now. |
| [`files.rs`](./files.rs) | Owner-only (0600 / 0700 on unix) writes of the local state files. |
| [`tools.rs`](./tools.rs) | The single `memory` agent tool and per-turn citations. |
| [`bus.rs`](./bus.rs) | The cron subscriber and the pending-deletions subscriber. |
| [`schemas.rs`](./schemas.rs), [`schemas/defs.rs`](./schemas/defs.rs), [`schemas/handlers.rs`](./schemas/handlers.rs) | The `openhuman.memory_*` controller schemas and thin handlers. |
| [`types.rs`](./types.rs) | RPC request and view shapes; contract types are re-exported from `tinymemory_api`. |
| [`error.rs`](./error.rs) | `MemoryError` and the seven stable error codes. |
| [`status.rs`](./status.rs) | The memory row of `subsystems.status`. |
| [`test_fixtures.rs`](./test_fixtures.rs) | Shared in-memory engines for the unit tests. |

## Key types and entry points

- `engine::resolve` / `engine::is_on` (`engine.rs`): the one way to get a
  `BoundEngine`, or learn that memory is off.
- `engine::install_host_engine` (`engine.rs`): lets an embedder or test
  supply its own `MemoryEngine`.
- `scope::within_agent` and `scope::resolve_current` (`scope.rs`): set and
  read the acting identity around a turn.
- `lifecycle::prefetch::pre_turn` and `compaction`
  (`lifecycle/prefetch.rs`): synchronous reads of completed packs plus scoped
  background refresh. `lifecycle::hooks` contains the underlying operations;
  `post_turn` is spawned after durable commit.
- `lifecycle::jobs::enqueue` and `run_due` (`lifecycle/jobs.rs`): the
  background queue.
- `MemoryTool` and `run_action` ([`tools.rs`](./tools.rs)): the `recall | fetch | learn |
  forget` tool. `learn` stamps the item with facts the model does not choose:
  workspace, thread id, memory agent id, the learnings namespace, the tool
  call, and `source.kind = agent`.
  `MemoryTool` bounds calls with [`tool_budget.rs`](./tool_budget.rs): 15 seconds
  per call, 30 seconds of aggregate reserved time and eight attempts per tracked
  harness run. Concurrent calls reserve from the same budget. The tracker holds
  128 run records and never evicts an outstanding reservation. Aggregate limits
  apply while the record is retained; idle eviction under churn can reset the
  record if that same run later calls memory again. A timeout disables further
  memory calls in the retained record and tells the model to continue without
  retrying; a new run starts fresh. Calls
  outside a harness still have the per-call deadline.
  Explicit `recall`/`fetch` await the engine within that budget. Agent
  `learn`/`forget` instead validate, confine and scrub their request into the
  private durable [`tool_writes.rs`](./tool_writes.rs) outbox, then start scoped
  background delivery. An acknowledgement says "queued; not yet saved to memory"
  or "queued; not yet removed from memory"; it confirms local admission, not a
  remote mutation. The model must not claim the write completed or repeat it to
  force indexing. A timed-out admission may already be durable, so it also must
  not be retried blindly. The queue holds at most 256 entries and 2 MiB per
  partition, retries under the matching owner on auth/background ticks, and
  keeps credentials out of persisted entries. Explicit RPC, UI, import and
  deletion workflows retain their own behavior.

- `deletion::forget_thread` (`deletion.rs`): called by `threads` when a
  thread is deleted. `channels::forget_channel` is called by `channels` on
  disconnect.
- `MemoryError` ([`error.rs`](./error.rs)): every RPC failure carries one of `MEMORY_OFF`,
  `UNSUPPORTED`, `INVALID_REQUEST`, `UNAUTHORIZED`, `INSUFFICIENT_CREDITS`,
  `UNAVAILABLE` or `ENGINE` in `data.code`. The two account-wide refusals are
  split out so a caller can tell "top up" and "try later" apart from an
  engine fault.

## RPC / CLI surface

All methods are `openhuman.memory_<function>`, listed in
`schemas/defs.rs::FUNCTIONS` and registered through `core/all.rs`.

| Group | Functions |
| --- | --- |
| Engine | `engines_list`, `engine_get`, `engine_set` |
| Recall policy | `policy_get`, `policy_set`, `pack_preview` |
| Items | `recall`, `fetch`, `learn`, `forget`, `erase_all`, `items_list`, `explore`, `items_get` |
| Agents and jobs | `agents_list`, `jobs_list`, `jobs_run` |
| Past chats | `conversations_backfill_status`, `conversations_backfill_start` |
| Brain | `brain_sources`, `brain_search`, `brain_ingest`, `brain_forget` |
| Sources | `sources_list`, `sources_add`, `sources_remove`, `sources_sync` |
| v1 import | `import_scan`, `import_start`, `import_status`, `import_retry_failed` |
| Layout migration | `migration_scan`, `migration_start`, `migration_status`, `migration_retry` |

## Persistence

There is no local memory database: items live in the engine. Local state
files under `<workspace>/memory/`, written owner-only through [`files.rs`](./files.rs):

| File | Owner |
| --- | --- |
| `jobs.json` | `lifecycle/jobs.rs` |
| `channel_threads.json` | `channels.rs` |
| `pending_deletions.json` | `deletion.rs` |
| `sources_state.json` | [`sources/state.rs`](./sources/state.rs) |
| `conversations_backfill.json` | `backfill.rs` |
| `import_state.json` | `import.rs` |
| `layout_migration.json` | [`layout_migration/state.rs`](./layout_migration/state.rs) |
| `local_root.json` | `local_root.rs` |

The sources registry is `[[memory.sources]]` in `config.toml`. The CortexDB
key is in the OS keyring. The layout migration's shared-tree claim marker
sits in the app directory shared by all accounts
(`<app>/memory/legacy_claims/`).

## Boundaries

- The engine contract, `AgentMemory`, pack rendering, belief building,
  explore facets, source readers, document converters and the engines are
  TinyMemory's (`vendor/tinymemory`: `tinymemory-api`, `tinymemory-tools`,
  `tinymemory-integrations`, `tinymemory-import`; the CortexDB engine is in
  its nested `tinycortex`). Fix engine or lifecycle behavior there.
- Turn delivery belongs to the agent harness: the session host prelude
  (`agent/session_host/runtime_session/memory_ingest.rs`), the pack
  middleware and the compaction summarizer live under `agent/`.
- Chat transcripts are owned by `threads`; this folder only reads them for
  backfill and forgets their memory on delete.
- Credentials come from `security::credentials` and the host's session
  owner. The core does not sign in or refresh anything here.
- Cron scheduling is `cron`'s; this folder only handles the due events.

## Gotchas

- The acting identity is a task-local. Work spawned off a turn without
  `scope::within` runs as the config's root identity, not the agent.
- Turn hooks run under the session's own config, which may be a derived
  context carrying a host binding (`[memory] agent_id` / `root`). Loading the
  global config instead would write to the wrong memory.
- Every write must go through a bound engine so the scrubber sees it. Do not
  hold a raw `MemoryEngine` outside `engine.rs`.
- RPC handlers must call through `confine.rs`, not [`ops.rs`](./ops.rs) directly, or a
  caller can read another root's memory.
- A new local state file should be written with `files::write_private`.
- `engine::invalidate` clears the engine cache; tests that swap engines rely
  on the `CACHE_STABLE` lock and `install_test_engine`.

## Tests

Unit tests sit beside each module as `<module>_tests.rs`, using the
in-memory engines in [`test_fixtures.rs`](./test_fixtures.rs). Run them with
`cargo test -p openhuman memory::` or `pnpm debug rust memory`. Integration
coverage is in `tests/memory_v2_e2e.rs` (JSON-RPC and a full web-chat turn
against the mock backend, run with `cargo test -p openhuman-cli --test
memory_v2_e2e`) and `tests/memory_cortexdb_live.rs` (live CortexDB, opt-in).
The UI flow is `app/test/playwright/specs/memory-v2.spec.ts`.

## Further reading

- [Memory (product)](../../../../gitbooks/features/memory.md)
- [Memory architecture](../../../../gitbooks/developing/architecture/memory.md)
- [Memory tools](../../../../gitbooks/features/native-tools/memory-tools.md)
- [tinymemory submodule](../../../../vendor/tinymemory/README.md)

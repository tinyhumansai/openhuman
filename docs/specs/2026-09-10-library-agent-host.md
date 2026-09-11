# OpenHuman library host: one core, many agents

**Status:** in progress — library identity and JSONL lock sharding implemented;
the typed per-agent resource API and RSS gate remain proposed.

## Objective

OpenHuman must be usable as an in-process execution library by products such as
OpenCompany. One host starts one core and then creates many independently
configured agent handles over it. The first capacity target is 100 live agent
handles and 100 concurrent turns in one process, without an OpenHuman account or
login flow and without duplicating core-wide services per agent.

The host supplies all product policy and resources programmatically:

- provider id, OpenAI-compatible URL, credential, model and model capabilities;
- agent definition and system prompt;
- skills and MCP servers, including per-agent allowlists;
- workspace/action root, tool policy, access policy and persistence policy;
- optional memory, transcript and event sinks.

Library mode must never discover an operator's OpenHuman login, configuration,
skills, MCP servers or workspace as an implicit fallback.

## Current state

The existing embedding surface already provides two important pieces:

- `CoreBuilder` can start an in-process `CoreRuntime` without HTTP or background
  services.
- `embed::Core::agent()` can run concurrent routed turns against that runtime.

It does not yet provide the required ownership model:

1. `HarnessBuilder` owns and builds a core. `HARNESS_LIVE` consequently permits
   one harness per process. Removing the atomic is unsafe because several core
   resources are still process-scoped.
2. Resolved: `HostKind::Library` bypasses the desktop registration check for a
   routed caller-supplied provider. The default harness no longer writes a fake
   `Session::local` auth state.
3. `TurnRequest` can select a route and model but not an agent definition.
   `inference::local::ops::agent_chat` always builds the orchestrator.
4. Agent definitions come from the process-global `AgentDefinitionRegistry`;
   skills are copied into a workspace; MCP servers are installed into the
   core-wide static registry. Those are discovery/configuration mechanisms, not
   per-agent dependency injection.
5. Every turn constructs a full `Agent`, including its tool objects, prompt
   machinery, provider selection and session adapters.
6. Resolved for independent transcripts: OpenHuman's JSONL conversation store
   now uses a per-root lifecycle lock, per-root metadata lock and per-thread
   message locks. Shared `threads.jsonl` metadata fsyncs remain serialized
   within one root.

The desired model is therefore **not** 100 `Harness` or `CoreRuntime` values.
It is one library host with 100 cheap agent handles.

## Public API

Names below describe ownership and are not required to preserve the current
`Harness` naming.

```rust
let host = LibraryHost::builder()
    .workspace(host_workspace)
    .persistence(Persistence::Sqlite)
    .limits(HostLimits {
        max_concurrent_turns: 100,
        max_blocking_tasks: 16,
        ..Default::default()
    })
    .build()
    .await?;

let analyst = host.agent(
    AgentSpec::new("company-analyst")
        .prompt(analyst_prompt)
        .provider(
            ProviderSpec::openai_compatible(
                "opencompany-primary",
                provider_url,
                provider_credential,
            )
            .model("gpt-5"),
        )
        .skills(analyst_skills)
        .mcp_servers(company_mcp_servers)
        .tool_policy(analyst_tool_policy)
        .memory(MemoryPolicy::SharedNamespace("company".into())),
)?;

let session = analyst.session(SessionSpec::new("customer-42:analysis-7"))?;
let result = session.turn("Review this company").send().await?;
```

### Ownership

- `LibraryHost` owns the single `CoreRuntime`, shared provider transports,
  module runtimes, registries, stores, resource limits and shutdown.
- `AgentHandle` is an `Arc`-backed immutable compiled `AgentSpec`. Cloning it is
  cheap and does not open stores, spawn tasks or build a new HTTP client.
- `AgentSession` binds an agent to a conversation and mutable turn state. It
  serializes turns for its own session only. Different sessions run in
  parallel.
- `Turn` owns per-call overrides and cancellation. Dropping a handle never
  shuts down the host.

`LibraryHost`, `AgentHandle` and `AgentSession` must be `Send + Sync`.

### Provider input

`ProviderSpec` requires a stable provider id, endpoint, credential and model.
The provider id is not inferred from the URL and is used for pooling,
rate-limit state, metrics and redacted diagnostics. Credentials are stored in a
redacting secret type and are never written to `Config`, auth profiles or the
workspace.

A provider instance/cache is keyed by the fields that affect transport and
model behavior. Agents sharing a provider reuse its HTTP connection pool,
retry/rate-limit state and model profile.

### Definitions, skills and MCP

The host accepts values, not only filesystem locations:

- `AgentSpec` owns an `AgentDefinition` value. It never needs insertion into the
  process-global definition registry for a root turn.
- `SkillSource` supports an already parsed skill bundle and an explicitly named
  directory. Directory loading occurs once during agent compilation. Skills are
  immutable after the handle is returned.
- `McpServerSpec` is supplied as data. The host owns one connection per distinct
  server identity and agents hold allowlisted views over its tool catalog.

Sub-agent definitions form an immutable registry owned by the compiled agent
graph. The existing global registry remains a compatibility path for the
desktop, but library turns do not read it.

## Authentication and host policy

Add an explicit library host policy to `CoreContext`; do not identify library
mode by the absence of a token or by a magic local session.

```rust
pub enum HostKind {
    TauriShell,
    Cli,
    Docker,
    Library,
}
```

For `HostKind::Library`:

- routed providers do not call `verify_session_active`;
- no auth session is created, read, refreshed or cleared;
- managed OpenHuman inference and login-gated hosted APIs are unavailable
  unless the embedder explicitly installs a backend-auth port;
- provider credentials grant access only to their declared provider;
- tool authorization still comes from the supplied access/tool policy. Library
  mode is not an automatic `full` autonomy grant.

The bypass must consult the ambient `CoreContext` installed by dispatch. The
desktop, CLI and Docker paths retain their current registration gate exactly.

## Runtime composition

Introduce a minimal `DomainSet::library()` and `ServiceSet::library()` rather
than using `DomainSet::embedded()`, which currently includes Medulla, flows,
channels, integrations and automation for a different embedding use case.

The default library set includes only the kernel needed for agent turns:

- agent, config, security and inference;
- threads and memory only when the selected persistence/memory policy needs
  them;
- skills, MCP and runtimes only when at least one agent declares them;
- no desktop, hosted, Medulla, channels, cron, heartbeat, update, Socket.IO or
  HTTP server.

Core-wide services are initialized once. Agent construction must not start a
background service.

## Concurrency rules

The API distinguishes **live handles** from **in-flight turns**. One hundred
live handles should add only their compiled prompts, tool views and small
session metadata. In-flight turns additionally consume model context, tool
state and response buffers.

- A per-session async mutex prevents two turns from racing the same transcript
  and conversation state.
- No mutex spans provider I/O or tool execution across different sessions.
- A host semaphore provides configurable global backpressure. Reaching the
  limit waits or returns a typed overload error; it never creates unbounded
  queues.
- Per-provider semaphores support upstream rate limits without blocking other
  providers.
- Cancellation propagates through provider calls, tool execution and queued
  persistence work.

The default limit should be conservative and configurable. Supporting 100
concurrent turns means the implementation remains correct at 100; it does not
mean every machine should default to 100 simultaneous provider streams.

## Persistence

The JSONL conversation store no longer has a process-wide mutex: lifecycle is
coordinated per root, shared metadata per root, and transcript IO per thread.
Every message append still fsyncs shared `threads.jsonl` metadata, so SQLite WAL
remains the preferred durable path for sustained high write throughput.

The library path should use a store contract with two initial implementations:

- `InMemory`, for ephemeral agents and tests;
- SQLite in WAL mode, for durable sessions.

SQLite uses a small bounded connection pool, transactional message + thread
metadata writes, indexed thread reads and busy timeouts. WAL allows concurrent
readers while one short write transaction commits. Pool size is a host limit,
not proportional to the number of agents.

The desktop JSONL implementation may remain during migration, but the store
must be selected through the existing session-history/store seams rather than
by `HostKind` checks scattered through the turn path. A compatibility importer
can move old JSONL transcripts into SQLite; dual-writing is not part of the hot
path.

The immediate lock fix is implemented and stress-tested with 100 distinct
threads. It also shards message-file IO by thread, beyond merely keying the old
mutex by root. It does not remove the same-workspace metadata serialization
point.

## Memory budget

The implementation is accepted on measured retained memory, not only type
shape. Add a benchmark executable that records RSS at these checkpoints:

1. process start;
2. library host built;
3. 100 distinct `AgentHandle`s built;
4. 100 sessions idle;
5. 100 turns blocked on a mock streaming provider;
6. all turns complete and transient state is dropped.

The benchmark uses a local mock provider and fixed prompts/tool catalogs so it
is deterministic. CI records the measurements; a dedicated performance lane
can enforce thresholds once stable baselines exist.

Initial targets on Linux x86_64 release builds:

- 100 idle agent handles and sessions add no more than 64 MiB over the built
  host;
- provider clients and MCP connections scale by unique provider/server, not by
  agent;
- completed turns return to within 10% of the pre-turn retained-memory level
  after caches have warmed;
- no Tokio runtime uses a 16 MiB stack per agent. Stack allocation is per
  worker thread, and worker count is explicitly configured by the host.

### 2026-09-10 baseline

The repaired `library-profile fleet` scenario constructed 100 shipped
orchestrator `Agent` values in one process, then ran one overlapping mock turn
per agent on four Tokio workers. On this Linux host, the optimized build
measured 19,904 KiB RSS before construction, 290,828 KiB after construction
(2,709.24 KiB marginal RSS per agent), and 477,336 KiB after the load phase.
The 100-turn latency was p50 11 ms, p95 16 ms, p99 19 ms, max 22 ms.

That baseline proves the current full `Agent` is too heavy to serve as the
public live handle: 100 idle instances add about 265 MiB, well above the 64 MiB
target. The proposed `AgentHandle` therefore needs to retain only a compiled
immutable spec and shared resources; constructing and parking 100 current
`Agent` values is not the intended final architecture.

## Delivery sequence

### Milestone 1: correct library identity and selectable agents

- Add `HostKind::Library` and remove the local-session requirement for routed
  library turns only.
- Add `agent_id` to the typed turn request and route it to
  `Agent::from_config_for_agent`.
- Add a cheap `Core::agent_named(id)`/`AgentHandle` facade over the existing
  runtime.
- Prove 100 concurrent mock-provider turns on distinct session ids complete on
  one core with no auth state.

This milestone can initially use the core-wide definition/skill/MCP registries;
it establishes correct lifecycle and concurrency without pretending injection
is finished.

### Milestone 2: injected immutable agent graphs

- Accept `AgentDefinition`, skills, MCP specs and policies as values.
- Compile and cache shared provider/tool/MCP resources at `host.agent(...)`.
- Stop rebuilding immutable tools and provider clients for every turn.
- Keep the desktop registry/config loaders as adapters into the same compiled
  representation.

### Milestone 3: scalable stores and measurement

- Add in-memory and SQLite-WAL session/conversation stores through the existing
  history/store seams.
- Keep measuring the sharded JSONL path and replace the remaining per-root
  metadata serialization with SQLite WAL where sustained throughput needs it.
- Add the RSS/concurrency benchmark and flamegraph allocation checkpoints.

### Milestone 4: API stabilization

- Run OpenCompany on the library API without environment mutation or fake auth.
- Document shutdown, cancellation, overload and secret-handling guarantees.
- Deprecate `HarnessBuilder::session(Session::local(..))` for library-routed
  providers after downstream migration.

## Acceptance tests

The feature is complete when all of the following are automated:

1. A library host runs a routed turn with no auth-profile files and no backend
   calls.
2. Desktop/CLI custom-provider calls still fail without their required app
   session.
3. Two injected agents with different prompts, providers, skills and MCP
   allowlists cannot observe or call each other's resources.
4. One hundred sessions run concurrently against a deterministic mock provider;
   every response and progress stream is correlated to the correct agent and
   session.
5. Concurrent turns in the same session serialize; turns in different sessions
   overlap.
6. Provider HTTP clients and MCP connections are constructed once per unique
   identity.
7. No process-wide conversation mutex is acquired by the scalable store path.
8. The RSS benchmark reports the checkpoints and satisfies the adopted budget.

## Non-goals

- Multiple independent `CoreRuntime`s or workspaces in one process. That is a
  different multi-context problem and is unnecessary for many agents sharing
  one OpenCompany host.
- Treating agents as security boundaries inside one process. Untrusted tenants
  still require OS/container isolation.
- Running local heavyweight inference models once per agent. Local engines, if
  enabled, are core-owned shared resources.
- Making all 100 turns unbounded. Backpressure is part of the contract.

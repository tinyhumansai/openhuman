# profiles

In SaaS mode (`core::runtime::Mode::Saas`) one process serves many users, and
each user is served as one **profile**. A profile is the tenant: one user's
config, credential, workspace, sandbox and conversation threads, one thread per
chat interface (web chat, and each hosted platform the gateway relays). This
domain maps a gateway user to that profile, lays out the profile's private
state, forces the config it runs with, leases it to one process, and keeps the
open profiles of the process.

A single-user core (desktop, CLI, `openhuman_embed::Runtime`) never serves it:
its controllers belong to `DomainGroup::Operator`, which only
`DomainSet::saas()` enables, and `host::host()` is `None` outside SaaS.

Two hosts drive it:

- `openhuman-core run --mode saas --saas-config <file>`: the JSON-RPC server
  behind a trusted gateway (`openhuman-rpc`'s `saas_gateway`). See
  [SaaS profiles](../../../../docs/gitbooks/en/developing/saas-profiles.md) for
  deploying it.
- `openhuman_embed::ProfileRuntime`: the same host in-process, without the
  gateway (`crates/openhuman-embed/src/profiles.rs`).

## Layout

```text
<root>/users/<profile-id>/          the desktop's users/<id> shape (config::schema::ProfileLayout)
  profile.toml                      ProfileMeta (only without a storage backend)
  config.toml                       the profile's config_path (never read in SaaS)
  auth-profiles.json                its credential (unless secrets live in the backend)
  workspace/                        threads, sessions, memory, cron, cost and run ledgers
  sandbox/                          action_dir: the only place its tools may act
<root>/users/<sha256(id)>/.lease    the file-lock lease and its record (only without a storage backend)
<root>/deprovisioned/<id>-<secs>-<uuid>/   an archived profile
<root>/operator/                    the operator plane's own state (operator_dir overrides it)
<root>/service.token                the gateway bearer (service_token_file overrides it)
```

With a storage backend (`storage_url` / `OPENHUMAN_STORAGE_URL`):

| Where | What |
| --- | --- |
| scope `cluster`, collection `profiles` | the profile registry (replaces `profile.toml`) |
| scope `cluster`, collection `leases` | the profile leases (replace the file locks) |
| scope `profile:<id>` (`storage::scope_for_profile`) | that profile's records on the storage ports: secrets and auth profiles, approvals, cron, flows, ... |
| session key `<id>~<agent or default>` (`tenant::session_key`) | its session store: transcripts, turn states |

A profile's files stay under `<root>/users/<id>/` either way, so the nodes of a
cluster share `<root>` on one volume (see [Known limits](#known-limits)).

The desktop config loader resolves a signed-in user's `users/<id>/config.toml`
and `workspace/` through the same `ProfileLayout`, so the two layouts cannot
drift. The older `<root>/agents/u-<hash>/` layout is gone; data left there is
not migrated.

## Profile ids

The top-level `profile_ids` setting in the operator config
(`profile_ids = "hashed"`, not under a table) picks how a gateway user id
becomes a `ProfileId`:

- `"raw"` (the default): a user id matching `^[a-z0-9][a-z0-9_-]{0,63}$` is
  used unchanged, so the desktop's 24-hex backend ids pass through. `local`,
  `operator` and anything starting with `h-` are reserved.
- Anything else, or every id under `"hashed"`, becomes `h-` plus the first 32
  hex characters of `sha256(user_id)`.

Provisioning also refuses a profile id equal to the file name of the node's
`operator_dir`: keyring secrets are namespaced by their store's directory name,
so such a profile would share the operator's credential slots.

Both forms fit the agent-id charset and never contain a path separator.
Changing the mode may re-map users onto different profiles. Gateway user ids
are turned into profile ids at once (`ops.rs`, `gateway.rs`) and are never
logged. Under raw mode a user id that fits the charset *is* the profile id, so
it reaches paths, storage and the provisioning response; use `"hashed"` where
user ids must not. `ProfileRuntime` may include the profile id in debug logs
during `provision` and `open`; do not treat debug logs as profile-id-free.

## The tenant key

The isolation boundary is the profile's own `CoreContext`, derived from the
operator's (`ProfileHost::open`) with:

- the forced config ([`layout::profile_config`](layout.rs));
- its own security policy (`agent_policy`, built over its workspace and
  sandbox), never the operator's live policy;
- `profile = <profile id>`: the **tenant key**;
- `session_agent = <profile id>` (see the rules below);
- fresh state slots, and the user families only (`host::user_domains`:
  threads, channels, memory), narrowed further by `surface::USER_METHODS`.

Work for a user runs under that context, and everything per-tenant keys on
`core::runtime::current_tenant()`, which in SaaS reads only the task's own
scope and fails closed (`NoTenant`) instead of falling back to the operator:

| What | Keyed by |
| --- | --- |
| storage backend records | `storage::scope_for_profile` |
| session store | `tenant::session_key` |
| web chat sessions, in-flight and parallel turns, background completions | `tenant::tenant_key(tenant, thread or request id)` |
| `/events` | `WebChannelEvent::profile`, stamped at publish, filtered per stream |
| cost ledger | the profile's own `CostTracker`, seeded at open |
| config loader | the context's config (`config/schema/load/saas_scope.rs`) |

So two users' `t1` are two threads, a cancel on one never stops the other,
and neither sees the other's events.

## Leases: one process per profile

Exactly one process hosts a profile at a time. `ProfileHost::open` takes the
profile's lease (`storage::lease`) before it opens it:

- **With a storage backend** it uses `DocumentLeases` in the backend's
  `cluster` scope: one compare-and-swapped record per profile with an owner
  (`node_id`), an endpoint (`advertise_url`), an epoch and an expiry
  `lease_ttl_secs` (default 30) out. Every node sharing the backend contends
  for the same records. A clustered node (one that sets `advertise_url`) must
  use a driver whose CAS holds across processes
  (`storage::driver_has_cross_process_cas`: SQLite, MongoDB); the boot guard
  refuses anything else.
- **Without one** it uses `LocalLeases`: an exclusive `flock` on
  `<root>/users/<sha256(id)>/.lease` (outside the profile's own directory,
  so archiving it leaves the lease in place), which keeps two processes on
  one root apart and drops when its holder dies.

What the lease drives:

- **Refusal.** A live lease held elsewhere makes `open` return
  `OpenError::HeldElsewhere(record)`. The gateway answers `409` (below);
  `ProfileRuntime::open` returns the error with the record.
- **Recovery.** A lease taken over from a holder that never released it
  (`previous_unclean`: it crashed or stopped renewing) means that holder's
  turns may have died mid-flight. `recover_workspace` marks them interrupted
  and settles orphaned run-ledger rows, and a storage-backed session store is
  swept for the profile too. A clean hand-over (eviction, release, shutdown)
  recovers nothing, so a re-open never interrupts live work. This is why the
  SaaS session store never recovers on open
  (`session_store::install_for_saas`).
- **Heartbeat.** `lease::heartbeat` renews every open profile's lease every
  third of the TTL. A renewal that finds the record changed (another node
  took over, or an operator released it), or that keeps failing until the
  grant would have expired, **fences** the profile: it is marked fenced, its
  in-flight turns are stopped (`web_chat::cancel_all_turns` under its
  context) and it is closed. `ensure_hosted` refuses new turns for a profile
  this node no longer hosts.
- **Write fencing.** Opening a profile registers a lease fence
  (`storage::fence`, built by `profiles::fence`) for its grant. Every write
  through the storage ports to the profile's scope or one of its session
  scopes is refused with a typed `FenceError` when the fence is latched, when
  the grant has run out by this node's clock less a skew margin (a sixth of
  the TTL), or when the stored lease record no longer names this node at this
  epoch (which latches it). A takeover therefore stops the old holder's next
  write, before its heartbeat notices. `ensure_hosted` refuses new turns on
  the latch or a locally expired grant, and the background loop checks the
  fence against the record before a profile's jobs and drops them if it
  latches mid-run. A latched fence keeps refusing for as long as the fenced
  profile's context is alive; a clean release retires it.
- **In use.** A profile is in use while anyone holds its `Profile` (a gateway
  request, a `ProfileHandle`) or a turn still runs on its context
  (`CoreContext::tenant_in_use`). A profile in use is never evicted, released
  or deprovisioned from under its work.
- **Release.** Idle eviction (`idle_evict_secs`, or LRU when
  `max_profiles_open` is reached), `profiles.release {profile_id}`,
  deprovisioning and a clean shutdown release the lease, so another node can
  take the profile at once. On shutdown a busy profile keeps its lease, so
  its successor treats the hand-over as unclean and recovers its turns.
- **Node ids** (`node_id`, else `OPENHUMAN_NODE_ID`, else random per process)
  must be unique among live nodes. A stable one lets a restarted node take its
  own profiles back at once instead of waiting out their leases.

## Lifecycle

Provisioning, opening, credential changes (`profiles.set_credential` /
`clear_credential`) and deprovisioning of one profile never interleave
(`lifecycle.rs`):

- **In one process**, each takes the profile's own lock (`ProfileLocks`, one
  async mutex per profile id, forgotten once nobody holds or waits for it),
  then the host's gate where it touches open profiles or leases. Different
  profiles do not wait for each other's lock; lookups of an open profile take
  neither. A credential change waits for an archive in flight and then finds
  the profile gone, so a late credential never lands in the keyring slot a
  re-provisioned profile would inherit.
- **Deprovisioning** closes the profile in the same step that finds it idle,
  so no request picks it up in between, and holds its lease from before the
  credential is cleared until the directory is archived and the record
  removed. It gives the lease back on every failure.
- **Provisioning a new profile** lays it out under its lease and refuses
  while another node holds it.
- **Opening** reads the registry again once it holds the lease: an open that
  raced a deprovision on another node answers `NotProvisioned` instead of
  recreating the archived directory.

Credential changes take no lease: the profile is normally hosted by some node
while the gateway installs a credential, and must not be refused for it. So
across nodes they are not ordered against a deprovision running elsewhere.
That only matters when secrets live in the storage backend; per-node
keyrings (see [Known limits](#known-limits)) never share a slot across nodes.

## Chat interfaces: one thread each

A profile holds one thread per conversation, whatever interface it came in on:

- **Web chat.** The user picks the thread id (`threads_upsert`,
  `channel_web_chat`): 1–128 characters of `[A-Za-z0-9_-]`. The core-reserved
  prefixes `channel:`, `proactive:` and `subagent:` are refused
  (`surface::validate_user_thread_id`).
- **Hosted platforms (relay).** The gateway owns the hosted Telegram, iMessage
  and Discord webhooks and account links. It relays each message in as its
  user with `channel_relay_inbound`
  ([`channels/providers/relay`](../channels/providers/relay/README.md)). The
  message lands on the user's `channel:<channel>/<sender>/<chat>` thread,
  which the user cannot mint themselves; its turn runs through the channel
  dispatch pipeline under the profile's context; each reply is published as a
  `channel_outbound` event on that user's `/events` stream for the gateway to
  deliver. A retried `message_id` runs nothing twice.
- **Deferred.** Native per-profile listeners (a user's own bot token) and
  several agents per profile need the host-agent resolver and
  `AgentContextRegistry` keyed by `(profile, agent)` first.

## Rules

- **A profile's config is forced, not configured.** `profile_config` is pure:
  `config.toml` is never read in SaaS, and storage needs no config document.
  - Every path sits under the profile's directory; artifacts land in its
    `sandbox/files`.
  - Memory is bound to the profile (`[memory] agent_id`, `root = user:<id>`).
    That binding wins over definition pins and team roots.
  - The memory layout is pinned to the legacy tree (confined to that root by
    `memory::user_scope`). Layout v3 would bind the engine below
    `memory::scope::user_root`, which reads `users/<id>` from the config path
    and would answer `org:<id>` or a minted local root instead.
  - The autonomy policy is on and supervised, with no auto-approval, no tool
    installation and no trusted roots.
- **The isolation boundary is the profile's `CoreContext`.** It carries the
  forced config, its own security policy (`agent_policy`, never the
  operator's), `profile = <profile id>` (the tenant key: storage scope,
  `/events`, cost, per-thread tables, via `core::runtime::current_tenant`)
  and the user families (threads, channels for web chat, memory), narrowed
  further by `surface::USER_METHODS`. Work for a user runs under it, which is
  what the config loader, the session store and the per-thread caches key on.
- **The default agent has no `session_agent`.** As on the desktop, a
  profile's default agent runs with no agent id: its state sits at the root
  of the profile's own workspace, and its session-store key is
  `<profile>~default` (`core::runtime::session_key`). Every seam that must
  keep users apart keys on the tenant, never on the agent id alone:
  - the approval gate's thread routes (`approval::gate::thread_route_key`:
    the `tenant_key` of the thread under a profile);
  - the MCP host (a profile's host lives at its workspace root; a profile
    never falls back to the process default or a lone open host, and a SaaS
    task with no scope gets none);
  - skill homes (`<workspace>/agents/default/` for a profile's default agent;
    never the operator's `~/.openhuman`, and nothing without a scope);
  - origin delivery and the transcript fallback (the profile's workspace
    root, the same place its file-backed session store writes);
  - web chat keys, in-flight turns, completion owners, event stamps
    (`WebChannelEvent::profile`), storage scopes and the cost ledger, which
    already keyed on the profile.

  What still reads the agent id (`agent_scope::current_agent_id`,
  `CoreContext::session_agent`) is genuinely per agent: `agent_scope_dir` (an
  embedded agent's subdirectory of a workspace), the cost ledger's
  `session_agent` column, owner records for embedded agents (devices, cron
  completions, flows) and log fields.
- **No ambient fallback.** Tenant-keyed code reads `current_tenant()`, never
  `CoreContext::current()` or `.session_agent()`; `pnpm saas:ambient`
  ratchets the reads that remain (`ambient-context`), alongside bare spawns,
  direct `load_or_init` calls, environment writes and `home_dir()` lookups.
  Detached work uses `spawn_scoped` so it keeps its profile.
- **Host tools are opt-in and confined.** A user's context has no `Platform`
  family. The standalone SaaS CLI has no host-tool adapter and exposes no
  shell or file tools, even when their group is in `tool_allowlist`. An
  embedding host must supply profile tool adapters and a permission callback;
  the allowlist limits which host groups those adapters may expose:
  - `host_files` (`file_read`, `file_write`, `edit`, `apply_patch`, `grep`,
    `glob`, `list`, `csv_export`, `read_workspace_state`) runs in-process,
    confined by the forced policy to the profile's `sandbox/`. In SaaS the
    policy grants neither `~/OpenHuman/projects` nor `/tmp/openhuman`, which
    every user would share.
  - `host_shell` runs every command in a fresh Docker container
    (`[sandbox]`: image, network, memory and CPU limits): read-only root
    filesystem, all capabilities dropped, no host environment, and the
    profile's `sandbox/` as its only writable mount. If the container cannot
    start, the command fails; it never falls back to the host.
  - Tools that change the process, install code or reach shared state are
    hard-denied whatever the allowlist says (`tools::HARD_DENIED`).
  - There is no per-user approval surface, so the approval gate never parks
    in SaaS: it allows tools from an allowlisted group and refuses the rest.
  - An embedding operator can also name `cron`, `current_time` and
    `resolve_time` in `tool_scope(ToolScopeSpec::Named(..))`
    (`tools::OPERATOR_ADMITTABLE`); only those three are registered despite
    their family being off, and only when named. Cron stays bound by
    `[cron] agent_jobs_only`. An embedded (library) agent gets the same three
    by naming them in its own definition's scope (`HostOverrides::named_tools`).
  - `max_tool_iterations` caps tool rounds per profile turn (chat, relay and
    cron), forced as `agent.max_tool_iterations_override`.
- **The user surface is an allowlist.** Within the user families, only the
  methods on `surface::USER_METHODS` are live: thread reads and writes
  (including `threads_delete` and `threads_purge`, confined to the caller),
  the turn-starting thread methods, web chat with cancel and queue control,
  `channel_relay_inbound`, and memory reads and writes confined to the user's
  tree. Anything unlisted answers as an unknown method and is absent from
  `/schema`. `DomainSet::saas()` registers the user families so profile
  contexts can derive them, and the surface gate keeps the operator scope on
  the operator plane, so the operator never serves user methods on its own
  workspace.
- **Deprovisioning archives.** The profile's directory moves to
  `<root>/deprovisioned/<id>-<unix-secs>-<uuid>/`; nothing is deleted. Its
  credential is cleared first (it lives in the keyring or the storage
  backend, not only in the directory), so a re-provisioned user does not
  inherit it. If clearing it fails the profile is kept, not archived, and its
  lease is given back so the retry can take it. A profile in use, or hosted
  by another node, is not archived: the call fails and can be retried.

## Gateway contract

```text
Authorization: Bearer <service token>                                  every request
X-OpenHuman-User: <gateway user id>                                    work for a user
X-OpenHuman-User-Sig: t=<unix secs>,v1=<hex hmac-sha256(service token, "<t>.<user id>")>
```

- No user header: the request runs on the operator plane (`profiles.*`).
- With one, `openhuman-rpc`'s `saas_gateway` layer handles it in this order:
  1. The bearer, **before** anything else (`401`). An unauthenticated caller
     cannot tell which users exist, and cannot open profiles.
  2. A duplicate or unreadable user header (`400`).
  3. The signature: ±60 s, bound to the user id (`401`). Required unless the
     operator sets `require_user_signature = false`.
  4. The user's profile, opened and leased: `403` when it is not provisioned,
     `503` when every slot is busy or storage fails, and **`409`** when
     another node hosts it:

     ```text
     X-OpenHuman-Profile-Owner: <owner node id>

     {"error": "profile_held", "owner": "<node id>",
      "endpoint": "<owner's advertise_url, or null>", "retry_after_ms": <ms>}
     ```

     The gateway routes the user to `endpoint`, or retries after
     `retry_after_ms`; a dead owner's lease lapses within `lease_ttl_secs`.
  5. The request runs under that profile's context. A user's context cannot
     reach the operator plane.
- `GET /events?client_id=` with a user header streams only that profile's
  events (chat progress, `chat_done`, `channel_outbound`, ...). Without one it
  answers `404`, and browser bind tokens are not accepted in SaaS.
- Routes a SaaS core never serves answer `404`: `/v1`, `/events/*`, `/ws/*`,
  `/socket.io`, `/dev/connect` and `/oauth/*`.

## Credentials

The gateway installs each user's TinyHumans session JWT or API key with
`profiles.set_credential` (`ProfileRuntime::set_credential` in-process). It is
stored in the profile's own auth-profile store (scoped to the profile in a
storage backend), so any backend call made under that profile's context
resolves that user's credential and no other. The process-wide
`auth.set_credential` / `clear_credential` refuse in SaaS mode, because they
activate a user directory and rebind process globals. The core never
validates or echoes a credential. With `shared_backend_api_key`, users may
instead ride the operator's `OPENHUMAN_BACKEND_API_KEY`.

## Background work

A single-user core drains memory jobs from a cron system job behind the
process-wide scheduler gate. A SaaS core cannot use either: the cron service
is off, and the gate reflects the operator, who holds no credential. So
`background::spawn` (started by `saas::build`) runs one loop per process. Each
tick it sweeps idle profiles, then opens every provisioned profile whose
workspace has queued memory jobs (`memory::lifecycle::jobs::has_pending`) and
runs its due jobs under that profile's own context. Profiles with nothing
queued are not opened. User cron jobs are not served yet: the cron RPCs are
not on the user surface.

The prompt says `Host: hosted` instead of the server's hostname, and the
`## User` identity block stays empty, because no process-wide identity is ever
set.

## Files

| File | Purpose |
| --- | --- |
| `types.rs` | `ProfileId` and `ProfileIdMode`, `ProfileMeta`, and the operator-plane result types |
| `layout.rs` | The SaaS side of the shared `ProfileLayout`, archived profiles, and `profile_config`: the forced paths, memory binding (pinned to the legacy layout) and autonomy policy |
| `host.rs` | `ProfileHost`: provisioning, lazy open behind the lease, LRU and idle eviction (never of a profile in use), release, fencing, each profile's derived `CoreContext` and policy, `current()`, `ensure_hosted()` |
| `lifecycle.rs` | Provisioning, deprovisioning and credential changes, and `ProfileLocks`: the per-profile lock that keeps one profile's lifecycle operations from interleaving (see [Lifecycle](#lifecycle)) |
| `fence.rs` | The profile's lease fence: the scopes its grant guards (its own scope and every `<profile>~<agent>` session scope), the skew margin, and how the host registers, renews, latches and retires it |
| `recovery.rs` | Workspace and session-store recovery after an unclean takeover |
| `lease.rs` | The profile lease as the host uses it: `OpenError`, the lease store choice (`DocumentLeases` over a backend, else `LocalLeases`), and the heartbeat that renews every open profile's lease and fences the ones lost |
| `registry.rs` | `ProfileRegistry`: which profiles are provisioned, in the backend's `cluster` scope or as `profile.toml` files |
| `gateway.rs` | Which context a gateway request runs under: the operator plane, or the profile of the user named in `X-OpenHuman-User`, after the signature check; the typed refusal (`409` holder) |
| `surface.rs` | What a user may call: `USER_METHODS`, applied at dispatch, in the controller list and in `/schema`; user thread-id rules |
| `background.rs` | The SaaS background loop: idle sweeps and each profile's queued memory jobs under its context |
| `tools.rs` | Which agent tools a user gets: the host tool groups, the hard-deny list, the tool-list filter, the approval gate's SaaS verdict, and the container policy for a user's shell |
| `credentials.rs` | A profile's TinyHumans credential, stored beside its config or in the backend |
| `ops.rs` | `provision` / `deprovision` / `list` / `status` / `set_credential` / `clear_credential` / `release`, returning `Outcome<T>` |
| `schemas.rs` | The `profiles.*` controllers (`profile_id` in and out) |

Each file's tests sit beside it (`*_tests.rs`), including property suites for
ids (`types_proptest_tests.rs`) and the tenant key
(`core/runtime/tenant_proptest_tests.rs`). End to end:
`tests/saas_mode_e2e.rs` (the gateway, two users on one thread id, relayed
channel threads, `409`) and `crates/openhuman-embed/tests/saas_profiles.rs`
(the in-process `ProfileRuntime`).

## Known limits

- Thread JSONL, the local memory engine, the cost and run ledgers, background
  completions and the sandbox stay as files under `users/<id>/workspace`, so a
  multi-node deployment needs `<root>` on a shared read-write-many volume and
  a remote memory engine. Moving those onto the storage ports is a follow-up.
- On drivers with `Capability::Fencing` (memory, SQLite, file, MongoDB
  replica sets) the driver re-checks the lease record atomically with each
  write, so a takeover cuts off a paused holder's writes at storage. Where it
  cannot (MongoDB without transactions, MongoDB blobs, named databases) the
  host check and the write are two operations: a holder paused for longer
  than the skew margin right after a passing check can still land one write
  after a takeover (see `storage/README.md`).
- Only writes through the storage ports are fenced. Files a profile writes
  directly under `<root>/users/<id>/` (the memory job queue, the classic
  on-disk stores) are not; on a shared volume, stopping turns and jobs on
  fence is the mitigation.
- A node that re-opens a profile after being fenced registers a new fence;
  work left over from its older grant is stopped by turn cancellation, not by
  the fence.
- With file leases (no backend) nothing goes through the ports, and the
  `flock` cannot be lost while the process lives, so the fence only backs the
  latch checks.
- Each node keeps its own operator keyring (`operator_dir`), so a credential
  installed through one node is only readable there unless secrets live in
  the storage backend.
- Existing `agents/u-*` data is orphaned.

## In-process host capabilities

`openhuman_embed::ProfileRuntimeBuilder` accepts the existing `HostTools`
factory, `ToolScopeSpec::Named` (or `HostOnly`) and an asynchronous
`can_use_tool` callback. Registration contains exactly the selected admitted
native and host tools. A wildcard is refused. SaaS hard denies, host group
opt-ins, the profile lease fence and channel permission ceilings remain
stronger than the callback. With no callback, execution is denied.
Operations requiring supervised approval must use `Named`; `HostOnly` does
not provide the static operator allowlist used for that exemption.

The factory receives the exact durable chat or relay thread through
`TurnContext::session_id`; its ambient tenant
(`openhuman_embed::current_tenant`) is the owning profile. During
execution, `openhuman_embed::current_turn_origin` exposes the task's original
`AgentTurnOrigin`, including channel sender and reply target. Concurrent
messages do not mutate another turn's routing context.

Channel permissions default to read-only, including unknown channels. Hosts
must explicitly configure `web_channel` and their relayed channel to `Write`
or `Execute` for memory or cron mutation. This grants only a ceiling: a named
tool and the callback are still required. Ordinary public channel agents keep
their read-only posture. Dangerous profile ceilings are refused.

`memory_engine` supplies the existing `MemoryEngine` contract through each
profile's local adapters. Requests retain the forced `user:<profile-id>` root;
a mismatched config fails closed. The process-global host engine is still
ignored in SaaS. Resolution refuses a profile whose lease expired or was
latched, including queued workers resolving after lease loss. This is an
admission check: an external engine call already admitted can still finish
if lease loss happens while it is in flight; the generic `MemoryEngine`
contract does not provide atomic storage fencing.

Native assembly remains within the profile domain families registered at boot.
Currently that includes memory and threads; selecting another family does not
enable it. Provider credentials, operator-selected additional native domains
and scheduled profile turn resolution are separate capabilities; selecting a
native tool does not start a driver.

## Scheduled jobs (`cron.rs`)

A hosted profile has no scheduler of its own. When a host installs a
`cron::host_delivery::CronDelivery` (`ProfileRuntimeBuilder::cron_delivery` in
the embed crate), the background tick calls `cron::sweep` once a minute: for
every provisioned profile it reads the due agent jobs straight from the shared
storage backend (a closed profile is seen without being opened), opens the
profile, claims each `(job, scheduled time)` occurrence in the store
(`cron::claims`, an absent-only write with a lease, so two nodes never both run
it) and runs the claimed jobs under that profile's context with the ordinary
dispatcher. An `origin`-mode reply goes to the host with the profile id, the
thread the job was created in and its `JobOrigin`; `NO_REPLY` is never
delivered. Shell and flow jobs are not run. Without a shared backend there is
nothing to sweep.

On shutdown a sweep in flight is not awaited: its claims lapse after ten
minutes and another node may then run the interrupted occurrence again.

## One turn at a time per profile

Chat turns (`web_chat::run_chat_task`), relayed-message turns
(`run_relay_turn`) and the cron sweep's runs of one profile take that
profile's gate ([`turn_gate`](turn_gate.rs)) for the whole turn, so they
queue instead of overlapping. A due cron occurrence waits behind an in-flight
chat turn rather than being skipped; the sweep takes the gate before claiming
the occurrence so the wait does not use up the claim lease. Different
profiles never contend, and work with no profile (desktop) is not gated.

# storage

The process's storage backend on the
[tinystoragedrivers](https://github.com/tinyhumansai/tinystoragedrivers)
ports, vendored at `vendor/tinystoragedrivers` (v0.11.0). The host and every module declare it by git tag and the root and app `[patch]` tables redirect them all onto that checkout, so the build holds one copy (`scripts/ci/check-storage-owner.mjs` enforces it).

One URL picks it: `OPENHUMAN_STORAGE_URL`, else `[storage] url` in
`config.toml` (`config::StorageConfig`), else the default,
`sqlite:<workspace_dir>/storage` (`storage::config`). Every host opens that URL
at startup and installs it (`install_for_url`), and every store, the session
store included, reads from it. There is no second layout: what older installs
wrote (the per-domain `.db` files, JSONL transcripts and turn-state JSON) is
imported once, read-only, by `import/`. The `classic` opt-out is retired; the
value is read as unset, with a warning.

The default belongs to the **workspace**, so a different workspace is a
different database. When a different user signs in, `rebind::rebind` (called
first by `rebind_after_credential_change`) re-resolves the URL for the new
workspace, opens its backend, points the session store (`ReboundSessionStores`)
and the process slot at it together, then runs the importers over the new
workspace. An explicit URL names one backend for every user and is never
replaced; a failed reopen is an error and leaves the previous backend in place.

With no backend installed (a unit test, an embedder that skipped the host
boot, a SaaS core with no URL) a store opens the **same default** for its own
workspace on first use (`local::backend_for`), once per workspace for the life
of the process, so no store has a file-shaped implementation of its own.

| URL | Driver | Cargo feature |
| --- | --- | --- |
| `memory` | in-process, keeps nothing | always |
| `sqlite:<path>` (a `.db` file, or a directory with one file per database) | SQLite | `storage-sqlite` |
| `mongodb://…/<db>`, `mongodb+srv://…/<db>` | MongoDB, one database shared by every scope | `storage-mongodb` |
| `file:<dir>` | JSON and JSONL files | `storage-file` |

A URL for a driver the build lacks fails at `open`, naming the feature.
`storage-sqlite` is a default feature and in the shipped product; `file` is
always compiled in; `storage-mongodb` is in neither, a cloud build turns it
on.

## Legacy import

`import/` copies the legacy files into an installed backend once, read-only
(see the module doc and the architecture page): the four small stores'
SQLite files, the tinyflows stores, and the sessions. The
core calls `import::run` from `bootstrap_core_runtime` before any domain store
opens, and `rebind::rebind` calls `run_all` for a newly bound workspace;
`run_with` takes an explicit backend for tests. The tinyflows stores (cron
jobs with their runs and policies, the flow catalog with its drafts, and flow
checkpoints) are copied by `import::tinyflows` with `tinyflows-import`, which
keeps ids, writes only what is absent and reads copies of the files; a build
without the `flows` gate imports no cron jobs. Sessions (`import::sessions`) go
through TinyAgents' `legacy` importer, which owns those formats: the run ledger
from a `VACUUM INTO` snapshot of `session_db/sessions.db`, each
`session_raw` transcript into the scope of the agent its stem names (matched
against the known definitions, else the default agent), and the turn-state
JSON into the default agent. Each importer records a `tsd.migrations` entry per workspace, so a
record deleted afterwards is not resurrected from the untouched legacy file.

## Runtime state values

Small pieces of runtime state that used to be one JSON file each live in
`tsd.kv` through `state_kv::StateKv` (one `(owner, namespace)` per domain, one
entry per value, in the acting scope): the memory domain's `jobs`,
`channel_threads`, `pending_deletions`, `sources_state`, `import_state`,
`conversations_backfill`, `layout_migration`, `local_root` and tool-write
outbox partitions (`memory::state_store`), the sub-agent lifecycle records
(`agent::subagent_host::lifecycle_store`) and the computer site memory
(`modules::site_memory_host`). The first read in a backend copies the legacy
file once (`StateKv::import_file` / `import_once`: an existing entry wins, a
`tsd.migrations` entry stops a re-import from resurrecting a deleted value, the
file stays in place for the legacy archive step). A value a damaged file must
not reset (`layout_migration`, `local_root`, the tool-write outbox) uses the
strict import, which fails closed on an unparsable file.

## Layout migration

`layout/` moves a layout-v1 profile (`users/<id>/workspace/`) to v2
(`users/<id>/storage.db` + `files/`): lock, import the legacy files, migrate
the schema, `VACUUM INTO` the database (counts verified), move user files into
`files/`, write and verify `<root>/backups/<user>-<ts>.tar.zst`, then write the
`.layout` marker and delete only the archived originals. Progress is in
`users/<id>/.layout.state`; every step is idempotent, so a crash resumes.
Controllers: `storage.migrate {dry_run}`, `storage.status`,
`storage.restore {archive}`. `[storage] auto_migrate_layout = true` runs it at
boot. No-op for SaaS, an explicit storage URL (including MongoDB) and v2. Run
it with the core stopped.

## Entry points

- `configured_url(&Config)` / `url_from(env, &Config)`: the URL in effect.
- `open(url)`: parse and open a backend (credentials are redacted in logs) The
  driver connects on the process's storage runtime (two `openhuman-storage`
  worker threads, alive for the whole process), so MongoDB's connection-time
  pool and monitor tasks do not depend on the host runtime. Cleanup tasks
  registered by later object drops can run on the internal `storage::block_on`
  runtime instead.
- `install(backend)` / `installed()` / `clear()`: the process slot domains
  read the backend from.
- `scope_for_agent(agent_id)`: the storage scope (`local/agent:<id>`) an agent's records live
  under, the same mapping TinyAgents' `DriverSessionStores` uses, so every
  domain agrees.
- `driver_is_shared(driver)` / `installed_is_shared()`: whether other
  processes may write the same backend (MongoDB). Boot-time recovery, such
  as the orphaned-run sweep, is skipped on a shared backend.
- `backend_has_cross_process_cas(backend)`: whether the backend reports
  `Capability::SharedAcrossProcesses` (MongoDB, SQLite), i.e. a
  compare-and-swap atomic across processes, which a clustered node's leases
  need. `driver_has_cross_process_cas(driver)` is the same answer from a
  name, for the boot guard, which judges a URL before opening anything.
  Memory and file drivers coordinate only within one process.
- `blob_sweep`: reclaims expired blobs (`BlobStore::sweep_expired`)
  for `local`, each known agent scope and each provisioned SaaS profile
  (`u:<id>` plus the blob-owning modules' scopes for it) every hour,
  spawned with the cron `ServiceSet` switch (`core::runtime::services`; not
  gated on `[cron] enabled`, since it reclaims storage rather than running
  jobs). Each scope's sweep is bounded by `SCOPE_SWEEP_TIMEOUT`; a timeout
  counts as a failed sweep and is retried on the next tick.
- `sweep_unreferenced_blobs`: an explicit maintenance helper that pages
  physical blob metadata, keeps unknown or cutoff-equal ages, asks the caller
  whether each old key is referenced, and stops on the first error while
  preserving earlier deletions. It is not activated by the periodic sweeper:
  reference checking and deletion are separate operations, so a caller must
  coordinate blob writers/reference creation with cleanup or use immutable
  keys, with a grace period longer than the write-to-reference delay.
- `journal_maintenance`: invokes TinyAgents' `maintain_run_journals` with the
  host's configured retention and clock. It visits local and known agent
  scopes plus module scopes, and only profiles currently open under this
  process's live lease fence. Dormant profiles are handled after this host
  opens them; cluster scope is excluded. The successful one-shot inactive-run
  backfill marker is stored per scope in
  `openhuman.journal_backfill`. Journal sealing, active-run checks, and
  timestamp-fenced retention remain owner behavior.
- `fence` / `fenced_backend`: lease-epoch write fencing (see
  [Write fencing](#write-fencing)).
- `scope_for_profile(profile_id)`: the scope a SaaS profile's records live
  under (`u:<id>`; tsd hashes the segment when it is not valid).
- `current_scope()` / `current_scoped()`: the tenant's scope — its profile's
  when it serves one, else the acting agent's, else `local` on a single-user
  host; an error in SaaS mode without a profile — and the installed backend
  under it.
- `block_on(future)`: runs a storage future from synchronous store code on
  one shared runtime thread. The future depends only on that thread and the
  storage runtime, so callers blocked on it cannot starve the host runtime of
  the threads a backend needs.
- `documents::Repo` and `documents::compare_and_swap`: the base the domain
  stores build on. A `Repo` holds one domain's scoped document handle,
  declares its collections and runs each call on the blocking bridge;
  `compare_and_swap` is tsd's `Repo::update` with its default `RetryPolicy`,
  so the host keeps no retry loop of its own.
- `import_state`: where a one-time importer records that it finished
  (`tsd.migrations`, `Migrations::mark_done`) and the claim that serialises
  two processes (`Leases` on `tsd.leases`). `migrate_legacy_markers` turns the
  marker rows older builds wrote (`storage_import`, `threads_import`,
  `subagent_sessions_import`, `composio_state_import`, `*_host_import`) into
  the same entries at boot, so an installed profile is not imported again.
- `schema_support`: owner, time and id helpers the host domains' v1 -> v2
  steps share. Each host domain declares its own `storage_schema()` (owners
  `approvals`, `devices`, `notifications`, `task_sources`, `artifacts`, `cost`,
  `config`, `profiles`, `threads`, `subagents`, `agents`), composed in
  `schema.rs`. `host_importers` moves the v1 data whose v2 home is another
  owner's collection (`composio_state` into `tsd.kv`).

## Storage for loadable modules

A loaded module does not hold storage handles. It calls the bus interface
`ai.tinyhumans.tinystorage.Storage` (`modules::storage_host`), which answers
from the installed backend:

- the scope is derived by the host from the broker-authenticated caller (module
  id) and the host's current scope, never taken from the request (the wire types
  reject unknown fields, so a forged `scope` is a decode failure);
- the module's grant (`modules::storage_grants::StorageGrant`: collections with
  their schema, stream, blob and secret-name prefixes, read-only or writable)
  is enforced on every call, and only the host runs `ensure_collection`;
- the backend is the fenced one (`open` wraps it), and a profile's module scope
  sits under the profile's session-key prefix so its lease fence covers it;
- each call produces one audit line without a payload.

No module declares a grant yet, so none has storage. Bus streams for large
blobs and watch signals are not served yet.

## Leases

`storage::lease` gives one node exclusive, expiring ownership of a key; the
SaaS profile host uses it so exactly one core process serves a profile.
`LeaseStore` has four operations, each taking the caller's clock (`now_ms`)
so the rules never read the wall clock:

- `acquire(key, now_ms)` takes the key when it has no record, or its record
  is released, expired (`now_ms >= expires_at_ms`) or already this node's;
  otherwise `LeaseError::Held(record)` names the owner, its endpoint and
  (`retry_after_ms`) when to retry. Epochs start at 1 and every acquisition
  except a re-entrant one (the same store instance re-acquiring the epoch it
  holds) writes `epoch + 1`.
- `renew(grant, now_ms)` extends the latest grant its holder was issued at
  that epoch; a takeover, a release or a newer grant makes it
  `LeaseError::Lost`.
- `release(grant)` marks the record `released` and keeps it.
- `holder(key)` returns the stored record, live or not (`is_live(now_ms)`).

A grant's `previous_unclean` is set exactly when the acquisition replaced a
record that was not released and that this store instance did not hold: a
foreign holder that expired, or this node's own id left by a crashed earlier
process. The profile host runs workspace recovery on it. Node ids must be
unique among live processes.

| Store | Where | Use |
| --- | --- | --- |
| `DocumentLeases` | a thin adapter over tinystoragedrivers' `Leases` (`tsd.leases`, owner `profiles`, scope `cluster`); the record, CAS loop, epoch rules and fence are tsd's | clustered nodes on a driver with cross-process CAS |
| `LocalLeases` | an exclusive `fs2` flock on `<root>/<sha256(key) hex>/.lease`, record in `.lease.json` beside it; no expiry, the OS drops the lock when the process dies | hosts without a backend |

The v1 `leases` conversion is retained as a typed transform in
`profiles::storage_schema::upgrade_lease_records`. A host legacy-importer must
read the old profile lease records and import the transformed records into the
`openhuman.leases` collection in `cluster` scope before clustered profile
serving relies on them. The v2 boot path does not scan or mutate legacy
collections; until the host importer performs that cutover, old lease rows
remain untouched and are not visible to `DocumentLeases`.

Keys are 1 to 200 bytes of ASCII letters, digits and `- _ . @`, not starting
with `.`, so they are safe as directory names and document ids. The scope
`cluster` is shared by every node; an agent whose id is literally `cluster`
would map to the same scope (`scope_for_agent`).

Tests: `lease_tests.rs` (the acquire rule), `lease_documents_tests.rs`
(contention, expiry, takeover, stale renew, clean release, and an
eight-thread race on a SQLite file with `--features storage-sqlite`),
`lease_local_tests.rs` (two instances on one root, crashed holder), and
`lease_model_tests.rs`, a proptest state machine of random acquire, renew,
release, clock and crash steps across 2 to 4 nodes checked against a
reference model.

## Write fencing

`storage::fence` and `storage::fenced_backend` fence writes by lease epoch.
Every backend `open` returns is a `FencedBackend`: its scoped handles look
the scope up in the process's `FenceRegistry` on each write (document `put`,
`delete`, `delete_where`, `claim`, `atomic_batch`, `drop_collection`; stream
`append`, `append_batch`, `truncate_before`, `delete_stream`; blob `put`,
`delete`). A scope with no fence passes through, so single-user hosts see no
change. A scope with one runs `LeaseFence::check` first:

1. the latch (set by the heartbeat on a lost lease, or by an earlier check);
2. the grant's expiry by this node's clock, less a skew margin (no I/O);
3. the stored lease record must still name this node at this epoch,
   unreleased and unexpired; a moved record latches the fence, an
   unreadable one refuses the write without latching.

A write the host check admits then goes to the driver through
`StorageBackend::for_scope_fenced` with `LeaseFence::driver_fence`: the lease
record (`cluster` scope, `tsd.leases` collection, id = `Key::of(["profiles", <profile>])`)
must still match `epoch`, `holder` = this node and no `released` field. The driver checks
it in the same statement or transaction as the write (tinystoragedrivers
v0.5.0, `Capability::Fencing`), so a holder paused between its check and
its write cannot land the write after a takeover. The driver answers
`ErrorKind::Fenced`, and the host re-reads the record, latches the fence
and returns `FenceError::Superseded`.

Collection declarations (`ensure_collection`) are the exception: every
conditioned handle a `FencedBackend` returns, for a profile lease or a
caller's condition such as an import claim, sends them through the scope's
unconditioned handle (`declaring_ports`). MongoDB refuses a declaration on a
conditioned handle (`Unsupported(ConditionalWrites)`): it cannot join the
condition's transaction. A declaration is an idempotent schema merge with
no owner data for the condition to protect. The host's local fence check
still runs first, so a node that has lost its lease does not declare.
Without this, every claimed import on MongoDB failed at its first
`import_state` call, and was retried and failed again on every boot.

A refusal is a `StorageError` of kind `Backend` (not retryable, so CAS loops
do not spin) whose source is the typed `FenceError`; `fence::fence_error`
recovers it. The lookup is per write rather than per bound scope because
the session store caches its handles across a profile's open, fence and
re-open.

The write runs after the host check alone (and the paused-holder race stays
open) where the driver cannot fence it:

- a driver without `Capability::Fencing`: MongoDB without transactions (a
  standalone server);
- a write the driver answers `Unsupported(Fencing)`: MongoDB blob writes
  (GridFS cannot join a transaction);
- a named database (`StorageBackend::database`), which cannot see the lease
  records in the default database;
- a lease store with no record in the ports (`LocalLeases`).

Tests: `fence_tests.rs` (matching, local expiry, latch, record checks,
registry replace/retire/prune), `fenced_backend_tests.rs` (node A loses the
lease to node B by a clock advance: A's writes on every port are refused,
B's succeed; a paused node refuses its own writes inside the margin; named
databases; a fence over `LocalLeases`; and the paused-holder race on the
memory and SQLite drivers: A's check passes, B takes over, and the driver
refuses A's write, plus the fallbacks for named databases and drivers
without fencing).

## Consumers

- The SaaS profile host (`profiles::lease`, `profiles::registry`): one
  lease per profile (`DocumentLeases` over the installed backend, else
  `LocalLeases` under `<root>/users`) and the profile registry, collection
  `profiles` in the same `cluster` scope. A lease taken over unclean runs
  the profile's workspace recovery; a lost one fences the profile. See
  `profiles/README.md`. The server opens the operator's storage URL before
  boot (`openhuman-rpc`'s `session_store::install_for_saas`);
  `openhuman_embed::ProfileRuntime` opens and installs it itself when no
  backend is installed yet.
- The session store: `openhuman_rpc::session_store::install_for_host` opens
  the configured backend before boot and installs `DriverSessionStores`
  over it. See that module's README.
- Domain stores on the document port, each in a `store_documents.rs` (the
  legacy SQLite reader lives in its `store_import.rs`):
  approvals (`security::approval`), paired devices (`security::devices`),
  notifications (`desktop::notifications`) and task sources
  (`integrations::task_sources`).
- Module-owned stores use scoped document callbacks rather than linking their
  implementation crates into the core. TinyFlows Catalog accesses the current
  backend through `flows::catalog_storage` and its `CatalogStorage` bus
  contract; the Schedule service accesses cron documents through
  `cron::documents_port`. The modules own their schemas, migrations, flow state,
  and run checkpoints. TinyFlows' `DriverCheckpointer` schema and migrations
  are declared by `tinyflows_drivers::graph_checkpoint_schema()`.
- `block_on_anyhow(future)`: `block_on` for those stores, whose errors are
  `anyhow::Error`.
- Approvals (`security::approval`), through `store_documents.rs`.
- Secrets (`storage::secrets`): the keyring's user secrets
  (`security::secret_storage::get` / `set` / `delete`) and the credential stores'
  files (`auth-profiles.json`, `http-credentials.json`) become encrypted
  documents (`DocumentSecrets`, `enc2:`) in the acting agent's scope. Each
  scope's data key is derived with HKDF-SHA256 from the keyring master key
  (`OPENHUMAN_KEYRING_MASTER_KEY` / `_FILE`, else the OS keychain); with no
  master key they fail closed. The config encryption key stays on the
  process keyring, because `config.toml` is loaded before any agent acts.
  The layout migration (`storage/layout/credentials.rs`) imports the profile's
  `auth-profiles.json` and `http-credentials.json` into the `local` scope's
  secrets, verifies the read-back, archives them (`profile/` in the archive,
  restored beside `config.toml`) and deletes them. It skips them (files stay,
  adopted lazily) when no master key is resolvable without minting one, or
  the file is not JSON; a stored record that does not decrypt with the current
  key fails the migration. The shared `secrets.enc`, `keyring/master.key` and
  `dev-keychain.json` are process-level, outside every profile's plan, and are
  never written into `storage.db`.

## Background work and agent scopes

Work done inside an agent's turn runs under that agent's `CoreContext`
(`session_agent`, set for embed agents; a SaaS profile's context carries
`profile` instead, and its records land in the profile's scope), so with a
backend installed its records land in that agent's scope. Background work
runs under the process default context and on its own would only see
`local`. `storage::agents` closes the gap:

- Known agents: the live ones are `core::runtime::AgentContextRegistry`'s
  (embed registers each agent it builds and deregisters it on drop).
  `AgentContextRegistry::register` also records the agent id in the
  backend's `local` scope (`storage_agents`, `agents::record`), so a
  restarted process still knows it; `agents::contexts()` lists both.
- `for_each_scope(label, step)`: runs `step` for `local`, then under each
  known agent's context — its live one, or the default context acting for it
  (`CoreContext::for_agent`). `for_each_agent` skips `local`.
- `within_agent(agent, fut)` / `context_for(agent)`: re-enter an agent's
  scope when background work learned whose record it is handling.
- `find_owner(label, probe)`: the scope (`local` first, then each agent)
  where a record named only by id lives — for event subscribers, whose
  events carry ids but no agent.

Users: the task-source poller, the flows boot sweep and schedule-trigger reconcile, the run reaper,
the device tunnel (a paired device's frames run as the agent that paired
it, `security::devices::owner`), and the event subscribers: a flow's
schedule tick, run digest and dedup settlement run as the flow's owner
(`flows::bus::owner`); Composio app-event triggers and new connections are
matched in every scope; a cron job's completion notification is stored with
the job's owner. Without a backend these run once, as
before. The cron scheduler visits live agents only
(`cron::scheduler::tick_live_agents`): an agent's jobs need its live
context (host tools, prompt) to run, so a recorded agent's jobs wait until
it is live again; with a backend it no longer needs the agent's `jobs.db`. In SaaS mode agent ids are not recorded and `local` is skipped;
per-user background work there is `profiles::background`.

## Boundaries

The ports, drivers, scopes and conformance suites are tinystoragedrivers';
the session store over them is `tinyagents-session`. This domain only
resolves the URL, opens the backend, and holds it for the process. The
`[storage]` section is bootstrap configuration and is never read from
storage itself.

## Schema, scopes and boot

- `scope.rs`: the one scope grammar (`tenant_scope`, `scope_for_profile`, `scope_for_agent`, `scope_for_module`), built on tsd's `Scope::tenant/local/agent/module`.
- `schema.rs`: `collection_owners()` validates linked crates' `CollectionSpec` declarations without merging owners; `module_collections(owner)` feeds module grants.
- `boot.rs`: `prepare_and_migrate`, called by `open`, ensures each linked crate's collections in the host's local and cluster scopes and runs the tinyagents graph and harness owner migrations there (a failure is logged and retried at the next boot). Tenant, agent and module scopes are prepared (`prepare_scope`, the same migrations included) by their owners when opened.
- `scope_move.rs`: the resumable copy of v1 scopes (`profile:`, `<profile>~<agent>`, raw agent ids, `module:<id>@<base>`, the `*-sha256:` fallbacks) into the v2 scopes.
- The generated reference is `docs/storage-schema.md`; `node scripts/ci/check-storage-schema.mjs` lints the current collection names, indexes, expiry fields and search fields.

---
description: >-
  The storage domain in crates/openhuman-core/src/storage/: the ports, URL
  resolution, Cargo features, per-agent scopes, drivers and on-disk layout.
icon: database
---

# Storage

OpenHuman keeps durable state on the ports of [`tinystoragedrivers`](https://github.com/tinyhumansai/tinystoragedrivers), vendored at `vendor/tinystoragedrivers`. The submodule ships a curated surface — types, API traits, bus contracts and the memory engine — plus separate engine crates enabled by feature flag. The root and app `[patch."https://github.com/tinyhumansai/tinystoragedrivers"]` tables redirect every declaration onto the one submodule checkout, so the build holds one copy. The vendored commit must stay semver-compatible with the tag modules declare; `scripts/ci/check-storage-owner.mjs` enforces two rules: (1) every `tinystoragedrivers*` package in the resolved graph is a path under `vendor/tinystoragedrivers/`, and (2) no non-root first-party crate reaches an engine crate (`rusqlite`, `mongodb`, `bson`, `aws-sdk-s3`, `aws-smithy-*`, `lapin`, `amq-protocol`, `object_store`, `sqlx`, `redb`, `sled`, `rocksdb`, `fs2`) through any normal or build dependency. The reasoned allowlist covers composition roots only (`openhuman`, `openhuman-app`, `openhuman-cli`, `openhuman-tui`, `openhuman-rpc`, `tinychannels`, `tinybox-cli`, `tinyjuice`). One URL picks the backend for the whole process. The host adapter is `crates/openhuman-core/src/storage/`. It resolves the URL, opens the backend, holds it in a process slot and decides which scope a call runs in. The ports, the drivers and their conformance suites belong to `tinystoragedrivers`, and the typed records stay with the domain that owns them.

With no URL configured, which is the desktop default, the host opens `sqlite:<workspace>/storage` and installs it. The URL decides where the whole process keeps its state, and there is no second layout: older installs' per-domain databases, transcripts and turn-state files are imported once, read-only, when the backend is installed. The files that remain outside the ports are listed under "What is not on the ports yet" below.

## The ports

A backend is a `StorageBackend`. It hands out a `ScopedStorage` for each `Scope`, and every handle on it is already bound to that scope, so no method takes a scope argument.

| Port            | Holds                                                                                                                                                      | Used for                                                                                                                    |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| `DocumentStore` | Versioned JSON documents in named collections, with filters, paging, unique indexes, compare-and-swap (`Precondition`), atomic `claim` and optional expiry | Approvals, devices, notifications, task sources, cron jobs and runs, the flow catalog, graph checkpoints                    |
| `StreamStore`   | Append-only logs with dense offsets                                                                                                                        | Transcripts, journals                                                                                                       |
| `KvStore`       | JSON values addressed by key                                                                                                                               | Small owner-managed values                                                                                                  |
| `BlobStore`     | Opaque bytes and trees by key                                                                                                                              | Attachments, artifacts and other large objects                                                                              |
| `QueueStore`    | Leased, acknowledged queue deliveries                                                                                                                      | Durable work queues                                                                                                         |
| `SecretStore`   | Scoped secret values, with optional conditional writes                                                                                                    | Secrets when the selected backend has a configured secret engine and key provider                                          |

The core's stores are mostly synchronous, so `storage::block_on` runs a storage future on one shared runtime thread. It never uses `block_in_place`, which panics on a current-thread runtime. `storage::documents::Repo` is the base the domain stores build on. It declares a domain's collections once per process and runs each call.

## Choosing a backend

The storage URL is the first of these that is set. Blank values count as unset.

1. `OPENHUMAN_STORAGE_URL`
2. `[storage] url` in `config.toml`
3. The default, `sqlite:<workspace>/storage` (one database per workspace)

The retired `classic` opt-out value is read as unset, with a warning.

`[storage]` is bootstrap configuration. It is always read from the environment or the file and never from storage itself.

| URL                                                                    | Driver                                                                                                                                                                                                               | Cargo feature     |
| ---------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------- |
| `memory`                                                               | In-process maps, keeps nothing                                                                                                                                                                                       | Always            |
| `sqlite:<path>` (a `.db` file, or a directory that holds `storage.db`) | SQLite                                                                                                                                                                                                               | `storage-sqlite`  |
| `file:<dir>`                                                           | `<dir>/scopes/<scope>/docs/<collection>/<id>.json` for documents, with collection metadata in `<dir>/_meta/collections/<collection>.json`. Streams are JSONL and blobs are raw files under the same scope directory. | `storage-file`    |
| `mongodb://…/<db>`, `mongodb+srv://…/<db>`                             | MongoDB, one database shared by every scope                                                                                                                                                                          | `storage-mongodb` |

A URL for a driver the build lacks fails at `storage::open`, and the error names the Cargo feature. A misconfigured deployment therefore stops at boot rather than at its first write. Credentials in a URL are redacted in logs and in `Debug` output.

Each feature is forwarded through the whole library chain (`openhuman-core`, `openhuman-embed`, `openhuman-tinyhumans`, `openhuman-rpc`, then the `openhuman-cli` and `openhuman-tui` hosts). `scripts/ci/check-feature-forwarding.mjs` checks the links. A desktop build never links a MongoDB client, and a cloud build does not need SQLite.

The host opens the configured backend before the core boots and installs it with `storage::install`. It then installs TinyAgents' `DriverSessionStores` over the same backend, so transcripts, turn states, the run ledger and journals live in it too. Library hosts use `RuntimeBuilder::storage(..)`, which installs only the backend: conversations move onto it with `RuntimeBuilder::session_store(..)`, configured separately.

## Scopes

Every record lives in a scope, and two scopes on one backend never see each other's data. There is one scope grammar, tinystoragedrivers' (`docs/specs/schema-v2.md` §1): a **tenant** (`local` for the desktop operator, `u:<profile>` for a SaaS profile) optionally narrowed to one **principal** (`<tenant>/agent:<id>` or `<tenant>/module:<id>`), plus the tenant-less `cluster` scope that holds the profile registry and the process leases. The host builds scopes only through `Scope::local()`, `Scope::tenant(id)`, `.agent(id)` and `.module(id)` (`storage::scope`: `tenant_scope`, `scope_for_profile`, `scope_for_agent`, `scope_for_module`), never from raw strings. A segment that is too long or holds characters outside the grammar is hashed by tsd itself (`~` plus 26 base32 characters), and only the segment is hashed, so a hashed agent or module still sits under its tenant.

The **tenant is the isolation boundary.** A profile's lease fence guards its tenant scope and the prefix `<tenant>/`, which covers every agent and module scope beneath it, hashed or not (`profiles::fence::scopes_of`). A tenant purge covers the tenant scope and every principal under it, enumerated with `StorageBackend::scopes_of_tenant`. The session store is TinyAgents' `DriverSessionStores`, one provider per tenant (`agent::session_store::TenantSessionStores` routes the host's flat session key `<profile>~<agent>` to the provider of that tenant), so a session record is always in `local/agent:<id>` or `u:<profile>/agent:<id>`. Domain stores keep their records in the tenant scope, and a loadable module's records are in `<tenant>/module:<id>`.

`storage::current_scope()` resolves the scope of the current call from the task's tenant (`storage::scope_from(profile, agent, saas)`), in this order:

1. A **profile**, when the work runs for one: the scope is the profile's, whatever agent acts inside it. In SaaS mode the agents of one profile therefore share that profile's scope.
2. Otherwise the acting **agent** (`CoreContext::session_agent`): the scope is `local/agent:<id>`.
3. Otherwise `local`: the operator, the CLI and the desktop shell in single-user mode.

In SaaS mode a call with no profile is refused instead of falling into a bucket other users could share, whether or not an agent acts. If the task's tenant cannot be resolved at all (`current_tenant()` fails), `current_scope()` returns that error in every mode instead of falling back to `local`.

A backend decides how it enforces the scope. SQLite and the file driver keep each scope's data apart, and `local` is the scope a single-user install uses. MongoDB injects a scope key into every filter and every index prefix.

### Process state

The [process-state inventory](process-state.md) classifies the named shared-core
singleton candidates and records their tenant, durability and routing boundaries.
It also identifies pending regressions and implementation work.

### Process caches

The connected-integration catalog belongs to the calling profile, or to the
operator in single-user mode. Agents using that operator's same configuration
and credentials share it. Its cache key
also binds the configuration, backend endpoint and credential identity, using
digests rather than storing those secrets in the key. Identical service
credentials do not make two profiles share a snapshot.

| Cache                | Isolation and lifetime                                                                                                                                                                                                             |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `INTEGRATIONS_CACHE` | Per-process snapshots keyed by profile/operator scope and credential identity. Invalidation and connection reconciliation affect only the calling scope. Entries are reconstructed from the backend; they are not durable records. |
| `CACHE_GENERATIONS`  | Per-process epochs for the same profile/operator scopes. Invalidation fences that scope's in-flight fetches without rejecting another profile's completed fetch.                                                                   |
| Storage `KEYS`       | The process master-key provider. Each secret operation derives its data key from the captured native storage scope; the cached value is not a tenant data key. Changing the master source requires a new process.                  |

SaaS work without a profile cannot read, populate, reconcile or invalidate the
integration cache. The Settings connection poll keeps its calling context when
it starts a background refresh with `spawn_scoped`.

### Background work

Background work runs under the process default context, which names no agent, and on its own would only see `local`. `storage::agents` closes the gap. Agents are known when they are live (`AgentContextRegistry`) or when an earlier process recorded their id in the `local` scope (`storage_agents`). `for_each_scope` runs a step once for `local` and once under each known agent. The cron scheduler's agent pass (`cron::scheduler::tick_live_agents`, driven in tests through the public `cron::scheduler::run_live_agent_pass`) runs each live agent's due jobs under that agent's own context, so their results are recorded in the agent's own scope. A recorded agent that is not live waits until it is live again, because its jobs need its host tools and prompt.

On a shared backend (MongoDB) other processes may write the same records, so boot-time recovery such as the orphaned-run sweep is skipped (`storage::installed_is_shared`).

### The storage runtime

`storage::open` connects the driver on a process-wide tokio runtime of its own (two `openhuman-storage` worker threads, never shut down). MongoDB's connection-time pool and server-monitor tasks live there and not on the host's runtime. Cleanup tasks registered by later object drops can run on the internal `storage::block_on` runtime instead. Synchronous stores reach the ports through `storage::block_on`, which parks the calling thread; with the backend on the storage runtime, any number of parked callers cannot leave the host runtime without the threads the backend needs to answer them. An embedder that wants this isolation passes its storage as a URL (`StorageSource::Url`) rather than a backend it opened on its own runtime.

## Drivers and on-disk layout

| Driver             | Where the data lives                                                                                                                                                                                                 |
| ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `memory`           | Nowhere. Records last as long as the backend value.                                                                                                                                                                  |
| `sqlite:<dir>`     | `<dir>/storage.db` in WAL mode (with its `-wal` and `-shm` files). The scope is a column on every row. The large relational stores keep their tables through the driver's native mode.                               |
| `sqlite:<file>.db` | That file, laid out the same way.                                                                                                                                                                                    |
| `file:<dir>`       | `<dir>/scopes/<scope>/docs/<collection>/<id>.json` for documents, with collection metadata in `<dir>/_meta/collections/<collection>.json`. Streams are JSONL and blobs are raw files under the same scope directory. |
| `mongodb://…/<db>` | One database. Each named database is a collection prefix, and every record carries its scope.                                                                                                                        |

### Profile layout v1 and v2

A desktop profile lives in `~/.openhuman/users/<id>/`, in one of two layouts (`config/schema/profile_layout.rs`, `LayoutVersion`):

```text
v1 (every install before storage schema v2)    v2 (a brand-new profile)
users/<id>/                                    users/<id>/
  config.toml                                    .layout        marker: 2
  workspace/    internal state + storage/        config.toml
                                                 storage.db     every store, one SQLite file
                                                 files/         Config.workspace_dir
```

The version is read from disk, never configured. A `.layout` file reading `2` is v2. A profile directory that is missing or empty is new and starts as v2 (the marker is written the first time it is resolved, so it stays v2 once `config.toml` appears). A directory with content and no marker is v1 and behaves exactly as before: `workspace_dir` is `users/<id>/workspace`, and the default URL is `sqlite:<workspace>/storage`. Resolving a v1 profile writes nothing.

Under v2 `Config.workspace_dir` is `users/<id>/files`, so the call sites that join workspace-relative paths keep working, and `config.toml` and `storage.db` stay at the profile root, where `workspace_dir.parent()` finds them. The default URL is `sqlite:users/<id>/storage.db`. An explicit `OPENHUMAN_STORAGE_URL` or `[storage] url` wins under both layouts. Rebind (a different user signing in) and the no-backend local fallback derive the URL from the new workspace, so they follow the layout. The binary reads both layouts. SaaS profiles (`ProfileLayout::new`) stay v1. Moving a v1 profile to v2 is the migration tracked in #7997 and is not done at boot.

### The default follows the workspace

The default URL is rooted at the workspace, so a different workspace is a different database. The desktop rebinds its workspace when a different user signs in. `storage::rebind` then re-resolves the URL for the new workspace, opens its backend, points the session store (`ReboundSessionStores`) and the process slot at it together, and runs the legacy importers over it. This happens before any other store is rebound, and a failed reopen is an error that leaves the previous backend installed rather than continuing on it. An explicit URL names one backend for every user and is never replaced.

A SaaS core with no URL keeps one default database per profile workspace, and its profiles never share a scope.

### What is not on the ports yet

The keyring or `secrets.enc` and the path-bound artifact bytes and media still use files. Cost tracking, the Composio state, sub-agent sessions and agent artifact records have no file fallback: with no backend installed they use the workspace's own default (`sqlite:<workspace>/storage`), the same as a host would, and a legacy cost ledger, artifact `meta.json`, Composio state file or sub-agent session file is imported once (and the file renamed to `*.migrated`). The vault watcher's mtime table stays a per-machine SQLite file on purpose, because its rows are absolute paths on one host. On first read with a backend installed, the auth-profile and HTTP-credential stores copy the records in their files into the backend, and the keyring adopts a secret it finds in the process backend. The source files are left untouched.

### Importing the legacy small stores

When the backend is installed, the core runs a one-time import at boot (`storage::import`, before any domain store opens). It covers the four small stores that keep one SQLite file each: approvals (`approval/approval.db`), devices (`devices/devices.db`), notifications (`notifications/notifications.db`) and task sources (`task_sources/sources.db`). The import is read-only. It copies the file, with its `-wal` and `-shm` companions, into a scratch directory and reads the copy, so the legacy reader's schema migration and the SQLite driver's table declaration never touch the real file. Both on-disk shapes are read: the original tables, and the driver document tables an earlier default wrote into the same file (those records win an id clash, being newer). Notifications for the core are re-filed under the real workspace's key.

Every record is written with an absent-only precondition, so an existing record is never overwritten. When a domain's records are written, a marker document (`storage_import/<domain>@<workspace digest>`, local scope) is stored, and a later boot does nothing. A deleted record is therefore not brought back from the untouched file. A failed import leaves no marker and is retried at the next boot; boot continues either way. SaaS processes have no workspace files and import nothing.

The tinyflows stores are imported the same way with TinyFlows' own reader (`tinyflows-import`): cron jobs with their runs and policies (`cron/jobs.db`, and each agent's `agents/<id>/cron/jobs.db`, filed under that agent's scope), and the flow catalog with its drafts and checkpoints (`flows/flows.db`, `flows/drafts/*.json`, `flows/checkpoints.db`). Records keep their ids, markers are `cron`, `cron/<agent>` and `flows`, and a build without the `flows` gate imports no cron jobs.

TinyFlows has no storage backend of its own any more, so these stores are on the ports whatever the URL says, and the legacy files are never modified or written again.

The host owns the legacy session sources and their lifecycle (`storage::import::sessions`). It snapshots `session_db/sessions.db` with `VACUUM INTO`, decodes its rows and the `session_raw` JSONL transcripts, resolves transcript agents from their stems, and maps the old rows into TinyAgents' typed import ports. The default agent receives the legacy database's run history and turn-state JSON; each transcript is written to the resolved agent's scope, falling back to the default agent when its identity is unknown. TinyAgents owns destination writes and its owner-local in-storage migrations, but does not open these source files. The marker is `sessions`; a run that skipped records leaves no marker and is retried.

Secrets with a backend installed are encrypted documents in the current storage scope: the acting agent's scope on a desktop or CLI host, and the profile's scope under SaaS, where every agent in one profile shares the profile's credentials and data key. Each scope's data key is derived with HKDF-SHA256 from the keyring master key (`OPENHUMAN_KEYRING_MASTER_KEY` or `_FILE`, else the OS keychain), and with no master key they fail closed. The config encryption key stays on the process keyring because `config.toml` is loaded before any agent acts.

## Tests and CI

The host's storage tests run on SQLite only. They test the host's wiring on a configured backend (which domain stores land in it, which scope they use, what a second process reads back), not the drivers. The shared helper is `tests/support/storage_case.rs`: it opens a SQLite backend on a temp directory and takes a lock on the process-wide storage slot, so the tests of one binary take turns.

Whether each driver honours the storage ports is proven upstream, in `tinystoragedrivers`. Its `conformance` module runs the same checks against every driver (nested scope isolation, compare-and-swap and leases across processes via `run_shared` for drivers that claim `Capability::SharedAcrossProcesses`, fencing, document secrets and migrations), and its CI runs them against a MongoDB replica set as well. The host asks a backend for `Capability::SharedAcrossProcesses` rather than hardcoding driver names where it can (`storage::backend_has_cross_process_cas`); the boot guard judges a URL before anything is opened, so it keeps a name check that a test pins to the real drivers.

The targets are `storage_approvals_e2e`, `storage_domains_e2e`, `storage_flows_e2e`, `storage_secrets_e2e`, `storage_scope_e2e`, `storage_agent_scopes_e2e`, `storage_desktop_upgrade_e2e` and `cli_storage_url_e2e`. Run one locally:

```bash
cargo test -p openhuman-cli --features storage-sqlite --test storage_approvals_e2e
```

CI runs these targets, with the lib tests of the domains on the ports, in the core coverage suite (`scripts/ci/rust-coverage.sh`). The `storage` area in `.github/ci-paths-filter.yml` arms it when storage, cron, flows, approvals, devices, notifications, task sources, keyring or credentials, orchestration, the cost tracker, app and desktop-control state or the rpc session store change. There is no separate driver lane and no MongoDB workflow in this repository. The `storage-mongodb` and `storage-file` features still ship in the product.

## Where to read next

- `crates/openhuman-core/src/storage/README.md` for the entry points and the consumers.
- The `tinystoragedrivers` specs: `docs/specs/storage-ports.md` and `docs/plans/storage-rollout.md` in the tinystoragedrivers repository (tag `v2.1.0`).
- [Security](security.md) for the keyring and credential stores.

## Schema, boot and migration

Storage crates expose pure v2 `CollectionSpec` values. The host validates and preserves those declarations by crate in `storage::schema::collection_owners`, then calls `ensure_collection` for each declaration in its shared scopes and when an owner opens a scope. The generated [`docs/storage-schema.md`](../../../../storage-schema.md) lists each crate's collection names, indexes, expiry fields and search fields; `scripts/ci/check-storage-schema.mjs` checks their names and millisecond timestamp fields. Record shapes, legacy conversion and migration rules remain with the owning domains.

`storage::open` runs `storage::boot::prepare_and_migrate` on the raw backend before the lease fence wraps it:

1. `collection_owners` validates each crate's declarations without combining owners.
2. The host ensures every crate's collections in its local and cluster scopes. Profile, agent and module scopes are prepared by their owners when opened; no driver-wide schema registry or scope enumeration is used.
3. Historical workspace and per-domain files are read by the separately gated `legacy-import` path. Its source formats and typed owner conversions are kept separate from normal backend opening.

A module's storage grant is generated from its owner's declared collections (`modules::storage_grant_schemas`), so it cannot drift from what the module declares.

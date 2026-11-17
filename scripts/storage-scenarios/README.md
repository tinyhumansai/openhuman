# Storage scenarios

How [tinystoragedrivers](../../vendor/tinystoragedrivers) works inside
OpenHuman, and whether data sent through the real core is stored correctly.
The script boots `openhuman-core serve`, drives data through the same JSON-RPC
methods the app calls, then checks it two ways: read back over RPC, and read
straight from the database or file tree. Every failed check is a finding for
a person to read; **nothing is fixed during a run**.

```bash
# every driver (sqlite, then file, then memory)
node scripts/storage-scenarios/run.mjs

# one driver, some scenarios (by id prefix; prerequisites run too, e.g. B, C,
# H and I bring A, F brings E)
node scripts/storage-scenarios/run.mjs --driver sqlite --only B,C

# re-render the report of an old run
node scripts/storage-scenarios/run.mjs --grade-only target/storage-scenarios/<run-id>
```

Requires a core build (`cargo build -p openhuman-cli --bin openhuman-core`) and
Node 24 (it reads SQLite with the built-in `node:sqlite`). It needs no account,
no network and no chat model: every record is written by an RPC, never by an
LLM turn.

The driver is [`run.mjs`](./run.mjs). The scenarios on the driver's own core
are in [`scenarios.mjs`](./scenarios.mjs), the ones that need a HOME of their
own in [`isolated.mjs`](./isolated.mjs), and the legacy-profile fixture in
[`legacy.mjs`](./legacy.mjs). The core rig (own HOME, JSON-RPC, scrubbed logs,
findings) is shared with [`memory-scenarios`](../memory-scenarios/lib.mjs).

## How storage works

What follows is read from the code (`crates/openhuman-core/src/storage/`,
`docs/gitbooks/en/developing/architecture/storage.md`); the scenarios below
check it.

**One backend per process, chosen by one URL.** The first that is set wins:
`OPENHUMAN_STORAGE_URL`, then `[storage] url` in `config.toml`, then the
default, a SQLite database that belongs to the active profile. The host opens
it before the core boots and installs it in a process slot
(`storage::install`); every domain store and TinyAgents' session store read
from that slot. A URL for a driver the build lacks fails at boot, naming the
Cargo feature.

| URL | Driver | Where the data is |
| --- | --- | --- |
| (default, profile layout v2) | SQLite | `~/.openhuman/users/<id>/storage.db` (+ `-wal`, `-shm`) |
| (default, profile layout v1) | SQLite | `~/.openhuman/users/<id>/workspace/storage/storage.db` |
| `sqlite:<dir or file.db>` | SQLite | that directory's `storage.db`, or that file |
| `file:<dir>` | files | `<dir>/scopes/<scope>/docs/<collection>/<id>.json`; streams as JSONL and blobs as raw files under the same scope directory |
| `memory` | in-process | nowhere: gone when the process exits |
| `mongodb://…/<db>` | MongoDB | one database for every scope (a cloud build only) |

The default follows the profile, so a different user is a different database:
signing another user in rebinds the backend to that user's file. An explicit
URL names one backend for every user and is never rebound.

**Six ports.** A backend hands out one handle per scope, and every port on the
handle is already bound to that scope.

| Port | Holds | Driven here by |
| --- | --- | --- |
| `DocumentStore` | versioned JSON documents in named collections, with compare-and-swap | `cron_add`, `notification_ingest`, `approval_preauthorize_flow`, `flows_create` / `flows_update` |
| `StreamStore` | append-only logs with dense offsets | `threads_message_append` (thread messages) |
| `BlobStore` | opaque bytes by key | `channel_web_voice_note_store` (`voice_notes/<id>.<ext>`) |
| `KvStore` | small JSON values by key | not drivable by RPC without a memory engine |
| `QueueStore` | leased queue deliveries | not driven |
| `SecretStore` | encrypted values | `auth_store_provider_credentials`, `auth_set_credential` |

On SQLite every port is a table with a `scope` column (`docs` holds
`(scope, coll, id, version, body)`, streams `(scope, name, seq, body)`, blobs
`(scope, key, bytes, sha256)`), so one file holds every scope and a query
always names one.

**Scopes.** Every record lives in exactly one scope, and two scopes never see
each other's data. A scope is a tenant (`local` for the desktop operator,
`u:<profile>` for a SaaS profile), optionally narrowed to a principal
(`local/agent:<id>`, `local/module:<id>`), plus `cluster` for process-wide
leases. The scope of a call is the profile's when it serves one, else the
acting agent's, else `local`; in SaaS mode a call with no profile is refused.
Domain stores (cron, notifications, approvals, threads, the flows catalog)
write to the acting scope, which for the desktop operator is `local`; a
session record is under `local/agent:<id>`.

**Secrets.** With a backend installed, a secret is an encrypted document
(collection `secrets`) in the acting scope. Each scope's data key is derived
with HKDF-SHA256 from the keyring master key (`OPENHUMAN_KEYRING_MASTER_KEY` or
`_FILE`, else the OS keychain); with no master key, secret writes fail closed.
Provider credentials are one secret (`file:auth-profiles.json`); the session
token is a profile inside it.

**Legacy import.** At boot, before any domain store opens, the core imports
what older releases kept in per-domain files: `approval/approval.db`,
`devices/devices.db`, `notifications/notifications.db`,
`task_sources/sources.db`, the tinyflows stores (`cron/jobs.db`,
`flows/flows.db`, drafts, checkpoints) and the sessions (`session_db/`,
`session_raw/*.jsonl`, turn states). The import reads copies, writes each
record only if absent, and stores a marker in `storage.import_state`
(`storage_import/<domain>@<workspace digest>`), so a later boot imports
nothing and a deleted record is never resurrected. The cost ledger
(`state/costs.jsonl`) is imported on first use and renamed to
`costs.jsonl.migrated`; `auth-profiles.json` is adopted into the secret on
first read and left in place.

**Layout v2.** A profile directory with content and no `.layout` marker is
layout v1 (`users/<id>/workspace/` holding everything). `[storage]
auto_migrate_layout = true` (or `storage.migrate` with the core stopped)
moves it to v2: it imports, `VACUUM INTO`s `users/<id>/storage.db`, moves user
files to `files/`, writes and verifies `backups/<user>-<ts>.tar.zst`, imports
the credential files when a master key is resolvable (and leaves them in place
when it is not, #8076), then writes the marker.

## Isolation and safety

- Each core runs with **its own `HOME`** under
  `target/storage-scenarios/<run-id>/<driver>/`; the operator's
  `OPENHUMAN_STORAGE_URL`, workspace, master key and backend credentials are
  removed from the environment before anything starts.
- The run directory is resolved with `realpath` first: the secrets backend
  refuses a directory with a symlinked ancestor, so a symlinked `target/`
  would otherwise fail every secrets check for a reason that is not the
  product's.
- Each core gets a **random master key**, passed through the environment only.
  It, every token the run mints and every fixture secret are scrubbed from
  `core.log` and `rpc.jsonl`.
- **No backend is reachable**: every config the run writes sets
  `api_url = "http://127.0.0.1:9"`, so a call the core makes on its own
  (notification triage, the first use of J's fictional API key) fails on a
  closed loopback port instead of reaching a network or a model, and
  `OPENHUMAN_AUTO_UPDATE_ENABLED=false` keeps the auto-updater from asking
  GitHub for a release.
- **No background job runs** (`[scheduler_gate] mode = "off"` in every config
  the run writes, including each signed-in user's): the legacy fixture holds
  cron jobs that are due. Composio is off.
- Users are signed in with an unsigned, locally minted session token; the core
  never validates it against a backend.
- **Every absence check has a positive control.** Before "not found" counts,
  the same query (or the same grep) must find a record that is there: the
  plaintext grep must find a stored message marker; bob must see his own
  records before "bob does not see alice's" passes; the memory driver must
  list a job written after the restart before "the old job is gone" passes.

## Scenarios

| id | drivers | what it checks |
| --- | --- | --- |
| A-write-read | all | per port: a cron job, a notification, a flow pre-authorisation (documents); a thread of 20 messages, including 64 KiB, unicode, JSON-ish and CR/LF bodies (stream); 4096 random bytes as a voice note (blob); a provider credential and a session token (secrets). Each read back byte-exact over RPC; the session token through `auth_get_session_token`, which returns it decrypted |
| B-durability | sqlite, file | everything from A survives a restart; appends in flight when the core is SIGKILLed: every acked message present after reboot, none torn, duplicated or phantom; `PRAGMA integrity_check` = ok (sqlite) or every JSON/JSONL record parses (file) |
| C-on-disk | sqlite, file | the database opened read-only: each record in the expected collection and scope (`local`); the blob's bytes; secrets are `secrets` documents with no plaintext; neither the provider token nor the session token appears anywhere under HOME |
| D-isolation | sqlite | two users: separate `storage.db` files; neither sees the other's cron job, thread, voice note or credential over RPC; neither profile directory holds the other's marker; switching back restores the first user's records |
| E-legacy-import | sqlite | a v1 profile seeded with every legacy store: imported (cron jobs, shell and agent ones with their ids and a flow-schedule job keyed `flow:<flow id>`; flows, notification, approvals, costs, auth profile), one marker per domain, originals byte-identical, the cost ledger renamed, the plaintext legacy token not in the database, and a second boot imports nothing |
| F-layout-migration | sqlite | E's profile moved to v2 by `auto_migrate_layout`: `storage_status` says 2, the marker, `storage.db` and `files/`, a non-empty `.tar.zst` backup, no collection shrank, the credential imported and the file archived; then a second v1 profile with **no master key**: the credential file stays byte-identical and its token reaches no database (#8076) |
| G-clear-app-data | sqlite | `config_reset_local_data` removes the active user's folder and `active_user.toml`, and leaves the other user's folder byte-identical (#8248); with an explicit storage URL, what survives the reset is reported |
| H-memory-restart | memory | A passes on the memory driver, and after a restart everything is gone, as designed |
| I-concurrency | sqlite | 8 `flows_update` calls with the same `expected_version`: exactly one wins, seven get `version_conflict`, the winner's name is stored, and the flow's scope; 50 parallel appends to one thread: none lost or doubled |
| J-signout-keeps-key | sqlite | session plus API key, then a session-only sign-out: `auth_get_state` is still authenticated by `api-key`, also after a restart, and the key is never on disk in plaintext (#8366) |

**Not driven** (said so in every report): the KV port (no RPC writes it
without a bound memory engine), agent and module principal scopes (they need
an agent turn or a module storage grant, and no module declares one), SaaS
tenants (they need the SaaS server mode), and artifact blobs (no RPC creates
an artifact).

## The fixtures are fictional

Tokens are random `storscen-…` strings, users are `storscen-<name>`, and the
legacy profile's tables carry the same placeholder rows as
`tests/storage_desktop_upgrade_e2e.rs`. The tinyflows stores are the committed
fixture in `crates/openhuman-core/src/storage/import/fixtures/tinyflows/`,
copied, never written to.

## Output

```
target/storage-scenarios/<run-id>/
  report.md       per driver: scenario table, findings, notes, what was not driven
  findings.json   [{id, engine (the driver), scenario, severity, expected, actual, evidence, basis}]
  checks.json     every check, passed or not
  results.json    measurements: per-scenario counts, the (scope, collection) census, scopes on disk, the crash and CAS numbers
  <driver>/core.log, <driver>/rpc.jsonl             the main core's log and every RPC (scrubbed)
  <driver>/iso-<name>/core.log, rpc.jsonl, home/    each isolated core
  <driver>/home/                                    the main core's HOME, left for a look
```

A finding's `basis` is **READ** when it was seen (an RPC answer, a row, a
file) and **INFERRED** when it was judged.

## See also

- [Storage architecture](../../docs/gitbooks/en/developing/architecture/storage.md) and the [storage domain README](../../crates/openhuman-core/src/storage/README.md).
- [`docs/storage-schema.md`](../../docs/storage-schema.md), the generated list of collections.
- The upstream ports and their conformance suites: [tinystoragedrivers](https://github.com/tinyhumansai/tinystoragedrivers), `docs/specs/storage-ports.md`.
- [`memory-scenarios`](../memory-scenarios/README.md), the same rig for memory.

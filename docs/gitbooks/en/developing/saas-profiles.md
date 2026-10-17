---
description: >-
  Running OpenHuman as a multi-user service: one profile per user, behind a
  trusted gateway, on one node or a cluster with leased profiles.
icon: users
---

# SaaS profiles

The OpenHuman core can serve many users from one process. Each user is one
**profile**: their own config, credential, workspace, sandbox and threads,
laid out like the desktop app's `users/<id>/` directory. A trusted gateway in
front of the core authenticates users and calls the core on their behalf. The
core never sees a user's password or login; it trusts the gateway's service
token and the user id the gateway names.

This page covers deploying that service. The in-process variant, for a Rust
server that wants the same isolation without a gateway, is
[`ProfileRuntime`](embed/concepts/profiles-saas.md).
The internals are in
[`profiles/README.md`](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-core/src/profiles/README.md).

## What a user gets

- **One profile per user.** It is opened on the user's first request, kept
  while in use, and closed after `idle_evict_secs` of idleness (or when
  `max_profiles_open` is reached and it is the least recently used idle one).
- **One thread per conversation, per interface.** Web chat threads are named
  by the user (`t1`, `work-notes`); a message the gateway relays from a
  hosted platform (Telegram, iMessage, Discord) lands on that user's
  `channel:<platform>/<sender>/<chat>` thread. Thread ids are unique per
  profile, so two users' `t1` never meet.
- **A reviewed surface.** A user's requests reach only the user methods:
  their threads, web chat (start, cancel, queue control), the channel relay,
  and reads and writes of their own memory. Everything else answers as an
  unknown method.
- **Their own events.** `GET /events?client_id=` streams only that user's
  chat progress, final replies and relayed `channel_outbound` replies.
- **Closed host tools.** The standalone SaaS CLI exposes no file or shell
  tools, even when `host_files` or `host_shell` is in `tool_allowlist`.
  Embedding hosts can supply profile tool adapters; the allowlist limits
  which host tool groups those adapters may expose.

## Operator config

The core runs in SaaS mode from the operator's own TOML file, never from a
user's `config.toml`:

```bash
openhuman-core run --mode saas --saas-config /etc/openhuman/saas.toml
```

```toml
root = "/srv/openhuman"              # absolute, existing, not world-writable
# service_token_file = "/srv/openhuman/service.token"   # the default
max_profiles_open = 256              # profiles open at once on this node
idle_evict_secs = 1800               # close a profile idle this long
profile_ids = "raw"                  # or "hashed"
require_user_signature = true        # X-OpenHuman-User-Sig on every user request
tool_allowlist = []                  # admission only; CLI has no host-tool adapter
shared_backend_api_key = true        # let every user ride OPENHUMAN_BACKEND_API_KEY
custom_definitions = false

# Cluster settings (see below)
# storage_url = "mongodb://db.internal/openhuman"   # OPENHUMAN_STORAGE_URL wins
# node_id = "node-a"                                # else OPENHUMAN_NODE_ID, else random
# advertise_url = "http://10.0.0.11:7788"           # marks the node as clustered
# lease_ttl_secs = 30
# operator_dir = "/var/lib/openhuman/node-a"        # default <root>/operator

[sandbox]                            # policy for host_shell supplied by an embedding host
image = "alpine:3.20"
network = "none"
memory_limit_mb = 512
cpu_limit = 1.0
```

`tool_allowlist` is an admission policy, not a tool installer. Its recognized
groups are `host_files` and `host_shell`. To expose those tools, use an
embedding host with profile tool adapters and a permission callback. Adding
either group to the standalone CLI configuration does not make those tools
available. The `host_shell` Docker boot check still applies when that group is
allowlisted.

Unknown keys are refused. The boot guard refuses to start, listing every
problem at once, when the root is relative, missing, world-writable or inside
`~/.openhuman`; when the service token is missing, readable by others or
shorter than 32 bytes; when single-user variables are set
(`OPENHUMAN_WORKSPACE`, `OPENHUMAN_CORE_TOKEN`, `OPENHUMAN_DEV_CONNECT`,
`OPENHUMAN_BACKEND_SESSION_TOKEN`) or a protection is switched off
(`OPENHUMAN_APPROVAL_GATE`, `OPENHUMAN_SANDBOX`); when `host_shell` is
allowlisted without a working Docker; and when a clustered node has no
storage backend that can keep leases exclusive.

Write the service token once, owner-only:

```bash
openssl rand -hex 32 > /srv/openhuman/service.token
chmod 600 /srv/openhuman/service.token
```

## The gateway contract

Every request carries the service token. A request for a user also names the
user and signs that name:

```text
Authorization: Bearer <service token>
X-OpenHuman-User: <your user id>
X-OpenHuman-User-Sig: t=<unix secs>,v1=<hex hmac-sha256(service token, "<t>.<user id>")>
```

- Without `X-OpenHuman-User` the request runs on the **operator plane**:
  `profiles.provision`, `deprovision`, `list`, `status`, `set_credential`,
  `clear_credential` and `release` (`openhuman.profiles_*` over JSON-RPC).
- With it, the core checks, in order: the bearer (`401`), a duplicate or
  unreadable user header (`400`), the signature, which must be within 60
  seconds of the core's clock (`401`), and the user's profile, which must
  already be provisioned (`403`). A full node or a storage failure answers
  `503`.
- The user's TinyHumans credential (session JWT or API key) goes in once
  through `profiles.set_credential`; the core uses it for that user's
  inference and backend calls and never validates or echoes it.

A minimal signer:

```python
import hashlib, hmac, time

def sign(service_token: str, user_id: str) -> str:
    t = int(time.time())
    tag = hmac.new(service_token.encode(), f"{t}.{user_id}".encode(), hashlib.sha256)
    return f"t={t},v1={tag.hexdigest()}"
```

### Profile ids

With `profile_ids = "raw"`, a user id matching `^[a-z0-9][a-z0-9_-]{0,63}$`
is the profile id as is (`local`, `operator` and `h-*` are reserved); any
other id becomes `h-<32 hex of sha256(user id)>`. `"hashed"` hashes every id,
which keeps your user ids out of paths and logs. Changing the mode maps users
onto new, empty profiles.

### Relaying hosted chat platforms

The gateway owns each platform's webhook and the user's account link. For
each inbound message it calls, as that user:

```json
{"method": "openhuman.channel_relay_inbound",
 "params": {"channel": "telegram", "chat_id": "777", "sender_id": "555",
            "sender_name": "Alice", "message_id": "tg-1", "text": "hello",
            "client_id": "channel-relay"}}
```

The answer comes at once (`accepted`, `thread_id`, `request_id`); a
`message_id` already recorded answers `duplicate: true` and runs nothing, so
retries are safe. The reply arrives on the user's
`/events?client_id=channel-relay` stream as a `channel_outbound` event
carrying `full_response` and the `channel` and `chat_id` to deliver it to.

## One node

On a single node, leave `storage_url` and `advertise_url` unset. The profile
registry is `profile.toml` files under `<root>/users/`, and each profile's
lease is a file lock on `<root>/users/<id>/.lease`, which also keeps a
second process on the same root from opening the same profile.

## A cluster

Several nodes can serve one user base. Each profile is hosted by exactly one
node at a time, enforced by a **lease** in a shared storage backend; any node
can host any profile.

1. **One storage backend.** Set `storage_url` (or `OPENHUMAN_STORAGE_URL`) to
   a driver whose compare-and-swap is atomic across processes: MongoDB
   (`mongodb://…/<db>`, the `storage-mongodb` build feature) for nodes on
   several hosts, or SQLite (`sqlite:<path>`, `storage-sqlite`) for several
   processes on one host. Do not put SQLite on a network filesystem. It holds the
   profile registry, the leases, the session store, and each profile's
   records (secrets, auth profiles, approvals, cron, flows) under scope
   `profile:<id>`.
2. **One shared volume for `root`.** Thread files, the cost and run ledgers,
   background completions and each user's `sandbox/` are still files under
   `<root>/users/<id>/`. Mount `root` on a read-write-many volume (NFS, EFS,
   CephFS) on every node. Keep memory on a remote engine: profiles use the
   hosted TinyHumans memory engine, which needs no volume.
3. **A unique, stable `node_id` per node,** and an `advertise_url` the gateway
   can reach it on. Setting `advertise_url` marks the node as clustered; the
   boot guard then insists on a cross-process backend. A stable node id lets
   a restarted node take its own profiles back at once.
4. **Its own `operator_dir` per node,** so nodes sharing a root do not share
   the operator workspace and keyring.

### Routing and the 409

Route users stickily: hash the user id onto a node (consistent hashing keeps
most users in place when nodes come and go). Stickiness is only for speed;
the lease is what makes it correct. When a request reaches a node while
another node holds that user's profile, the core answers:

```text
HTTP/1.1 409 Conflict
X-OpenHuman-Profile-Owner: node-b

{"error": "profile_held", "owner": "node-b",
 "endpoint": "http://10.0.0.12:7788", "retry_after_ms": 21500}
```

The gateway should forward the request to `endpoint` (and update its routing
for that user), or, when `endpoint` is null or unreachable, retry after
`retry_after_ms`. The `409` comes only after the bearer check, so an
unauthenticated caller learns nothing about which users exist.

To move a user deliberately (draining a node, rebalancing), call
`profiles.release {"profile_id": ...}` on the node that holds it. It refuses
while the profile is in use; retry shortly.

### Failover and recovery

- A holder renews each lease every third of `lease_ttl_secs`. A node that
  crashes or loses the backend stops renewing, and its profiles become
  available to other nodes once their leases expire (at most
  `lease_ttl_secs`, 30 seconds by default).
- The node that takes over a lease that was never released recovers the
  profile before serving it: turns that were in flight are marked
  interrupted and orphaned run-ledger rows are settled. A clean hand-over
  (idle eviction, release, shutdown) recovers nothing.
- A node that finds its lease changed under it (another node took over, or
  an operator released it) fences the profile: it stops that profile's
  running turns and starts no new ones.
- On a clean shutdown a node releases the leases of its idle profiles at
  once; a profile still busy keeps its lease, so its next holder recovers its
  turns.

Known limit: storage writes carry no epoch fence, so a node that is
partitioned from the backend but still running could write briefly after its
lease expired. Keep `lease_ttl_secs` well above network hiccups, and treat
the fence check before each turn and the turn cancellation on loss as the
mitigation.

## Checklist

- `root` absolute, not world-writable, on a shared RWX volume (cluster).
- `service.token` mode `0600`, at least 32 bytes, shared with the gateway only.
- Gateway signs `X-OpenHuman-User` and keeps clocks in sync (±60 s).
- `storage_url` on MongoDB (or SQLite for processes on one host), with the
  matching build feature (cluster).
- `node_id`, `advertise_url` and `operator_dir` unique per node (cluster).
- Load balancer hashes users stickily and honours `409` redirects.
- Provision a user (`profiles.provision`) and install their credential
  (`profiles.set_credential`) before their first request.

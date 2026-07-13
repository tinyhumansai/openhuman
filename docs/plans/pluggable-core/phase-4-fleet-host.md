# Phase 4 — Fleet supervisor: team/cloud hosting

**Status:** MVP **DONE** — `openhuman-fleet` binary (`src/bin/fleet.rs`,
`[[bin]] name = "openhuman-fleet"`). Remaining: backend membership sync,
ready-file port discovery, admin API for edge tokens (see "MVP vs production").

**Goal:** a supervisor (`openhuman-fleet`) that hosts one core per team member
and fronts them behind a single endpoint, so a team admin can
provision/manage members' assistants while every existing client
(`CloudHttpTransport`) keeps working unchanged.

## Delivered (MVP)

`src/bin/fleet.rs` — a self-contained binary (separate compile target, zero
weight on the shipped desktop/lib build, matching the `slack-backfill` bin
pattern):

- **Process-per-tenant MVP**: spawns `openhuman-core run --jsonrpc-only` per tenant
  with a per-user `OPENHUMAN_WORKSPACE`, a minted `OPENHUMAN_CORE_TOKEN`, and
  `OPENHUMAN_DISABLE_CHANNEL_LISTENERS=1` (this is the `ServiceSet::headless_api`
  shape from Phase 1). Before keeping a tenant registered, probes the assigned
  port through authenticated JSON-RPC with that tenant's core bearer; stale or
  fallback ports fail closed. Production multi-tenant security still requires
  distinct OS users or containers.
- **Reverse proxy**: axum `POST /{user_id}/rpc` forwards the JSON-RPC body
  verbatim to that tenant's core, swapping the client's **edge token** for the
  tenant's **core bearer** — the wire contract is unchanged end to end, so
  `CloudHttpTransport` works against `http://<fleet>/<user_id>/rpc`.
- **Edge auth / isolation**: distinct `EdgeToken` (client-facing) vs
  `CoreBearer` (fleet-only) types; the proxy rejects a token whose user does not
  match the path segment, so tenant A cannot reach tenant B's core. User ids are
  validated as single `[A-Za-z0-9_-]` segments (no path escape).
- **Tests**: pure logic unit-tested (port assignment + overflow, user-scoped
  workspace derivation, user-id validation, provisioning distinct
  ports/bearers/edge-tokens, edge-token→user round-trip, bearer parsing).

### MVP vs production (tracked follow-ups, logged not silently dropped)

- Ports are assigned sequentially from `--base-core-port`; production should read
  each core's actually-bound port from a ready file / `EmbeddedReadySignal`
  (Phase 1). The MVP checks loopback availability before spawn and requires an
  authenticated JSON-RPC readiness probe on the assigned port before keeping the
  tenant registered.
- Tenants come from `--users`; production reconciles membership against
  `tinyhumansai/backend` on a loop (same pattern as the cron scheduler).
- Minted edge tokens are never printed to stdout. The MVP can write them to a
  restricted operator-selected file with required `--edge-token-output`; production
  should expose them through an authenticated admin API.
- Container packaging (`HostKind::Docker`) and restart/backoff are not yet wired.

## Decision recap (README §2.3)

Process-per-tenant, not in-process multi-tenancy: it is the shape the
architecture already has (Tauri = one core per user), it sidesteps the
process-scoped items inventoried in phase 3 (env mutation for child tools,
keyring, Sentry), and it keeps the production security boundary explicit:
agents execute arbitrary tools, so real tenant separation must come from
distinct OS users or containers, not from a same-user supervisor process.

## Architecture

```text
client (CloudHttpTransport, per-user base URL + Bearer)
   │
   ▼
openhuman-fleet supervisor
   ├─ edge auth: mint/validate per-tenant session tokens
   ├─ reverse proxy  /:user_id/rpc  →  127.0.0.1:<that user's core port>
   ├─ membership sync ⇄ tinyhumansai/backend  (teams stay backend-truth)
   └─ lifecycle manager
        └─ per user: provision workspace volume →
           run core (child process or container, HostKind::Docker) with
           TokenSource::Fixed(per-tenant token), bind 127.0.0.1:0,
           ServiceSet::headless_api() + per-plan opt-ins (cron, heartbeat)
```

Key properties:

- **Wire contract unchanged** inside and out — the proxy strips
  `/:user_id` and forwards `POST /rpc` verbatim; `CloudHttpTransport`
  (`app/src/services/transport/CloudHttpTransport.ts`) already speaks this
  with a profile Bearer token and per-profile `rpcUrl`.
- **Port arbitration**: cores bind `:0`; supervisor reads the bound port from
  the existing `EmbeddedReadySignal` (child-process variant: ready line on
  stdout or a ready file — decide during implementation).
- **Teams**: membership/roles/invites remain in `tinyhumansai/backend`
  (`src/openhuman/team/` proxy untouched). The supervisor consumes the same
  backend API to decide _which_ cores exist and who may reach them; it adds
  hosting, not authorization semantics.
- **Secrets**: per-tenant token + per-tenant keyfile/env in the child's
  environment — no shared keyring across tenants (phase 3 inventory item).
- **Storage**: per-user workspace volume (`StorageBackend::WorkspaceFs`).
  This is why storage abstraction (phase 2.c) is off this phase's critical
  path; a managed-DB backend would slot in later as a new `StorageBackend`.
- **Health/restart**: existing health controllers over each core's `/rpc`;
  restart backoff via the existing `apply_startup_restart_delay_from_env`
  seam; container resource limits are the v1 noisy-neighbor answer.

## Scope

1. `openhuman-fleet` crate: tenant registry (backed by backend membership),
   lifecycle manager, token mint/validate, reverse proxy, health loop.
2. Container image + deployment doc for `HostKind::Docker` cores
   (`ServiceSet::headless_api()`).
3. Embedder guide in `gitbooks/developing/` covering all four consumption
   modes: Tauri embed, CLI one-shot, library (`AgentRuntime`), fleet.
4. E2E: supervisor + 2 tenants + `CloudHttpTransport`-shaped client script;
   assert tenant A's token cannot reach tenant B's core.

## Risks & mitigations

| Risk                                                               | Mitigation                                                                                                                                   |
| ------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------- |
| Token confusion between edge session tokens and core bearer tokens | supervisor is the only holder of core bearers; clients only ever see edge tokens; naming kept distinct in code (`EdgeToken` vs `CoreBearer`) |
| Resource blow-up (N cores × background services)                   | default `ServiceSet::headless_api()`; cron/heartbeat opt-in per plan tier                                                                    |
| Backend membership drift vs running fleet                          | reconcile loop (same pattern as cron scheduler); deprovision = stop core, retain volume per retention policy                                 |
| Supervisor as SPOF                                                 | stateless proxy + externalized tenant registry; ops concern, out of scope for v1 doc beyond noting it                                        |

## Out of scope (v1)

- In-process multi-workspace hosting (deferred behind phase 3; optimization
  for read-mostly tenants, never a security boundary).
- Managed-DB storage backend (needs phase 2.c traits; separate plan).
- Autoscaling / placement across machines.

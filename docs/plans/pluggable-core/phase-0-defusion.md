# Phase 0 — De-fusion cleanup

**Status:** planned.
**Goal:** pure motion inside `run_server_inner` so phase 1 is a composition
change, not a untangling change. No behavior change; verified by diffing
startup log lines.

## Scope

### 0.1 Delete the dead dispatch tier

`src/rpc/dispatch.rs:10-15` always returns `None` (module doc says the
registry is authoritative). Remove the module and its tier-3 call site in
`src/core/dispatch.rs::dispatch`. `src/rpc/` keeps `structured_error.rs`
(still used) — only the dispatch shim dies.

### 0.2 Extract inline service spawns

Each inline `tokio::spawn` block in `run_server_inner` becomes a named
function in a new `src/core/runtime/services.rs`:

| Service                                                                     | Today (approx.)         | Extracted fn                              |
| --------------------------------------------------------------------------- | ----------------------- | ----------------------------------------- |
| Heartbeat + subconscious bootstrap                                          | `jsonrpc.rs:~2083-2110` | `spawn_heartbeat_service(cancel, config)` |
| Update scheduler                                                            | `:~2113`                | `spawn_update_scheduler(cancel)`          |
| Cron scheduler (+ proactive-agent seeding, flow schedule-trigger reconcile) | `:~2124-2160`           | `spawn_cron_service(cancel, config)`      |
| Channel listeners                                                           | `:~2162-2191`           | `spawn_channels_service(cancel, config)`  |

Rules:

- Every extracted fn takes the `CancellationToken` explicitly (today some
  blocks capture it, some don't — normalize).
- Existing gates stay _inside_ the fns for now (`config.cron.enabled`,
  `config.heartbeat.enabled`, `OPENHUMAN_DISABLE_CHANNEL_LISTENERS`,
  `has_listening_integrations()`), so call sites don't change semantics.
  Phase 1 lifts the _selection_ (should this service exist) to `ServiceSet`
  while the fns keep their _config_ gates (is it enabled for this user).
- Keep the deliberate asymmetry documented at `jsonrpc.rs:2259-2264`: the
  flows trigger subscriber registers at unconditional core boot, not under
  channels — that stays in `bootstrap_core_runtime`, not in
  `spawn_channels_service`.

### 0.3 Extract the store-init block

`jsonrpc.rs:1807-1892` (memory, whatsapp_data, people, attachments global
inits) → `fn init_stores(config: &Config, workspace_dir: &Path) ->
anyhow::Result<()>` in `src/core/runtime/context.rs` (file seeded here,
grows in phase 1/2). Preserve init order exactly; comment each step with the
prior `jsonrpc.rs` line range so the diff is auditable.

## Out of scope

- Any signature or behavior change to the spawned services themselves.
- Touching `bootstrap_core_runtime` internals (phase 1 moves the call, phase
  2/3 decompose it).

## Verification

- `cargo check` both crates; `pnpm test:rust`.
- Boot the desktop app and `openhuman-core serve`; diff the startup log
  sequence (grep-stable prefixes) against `main` — must be identical.
- `rg 'rpc::try_dispatch|rpc/dispatch'` returns nothing.

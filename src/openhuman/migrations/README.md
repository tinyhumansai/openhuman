# migrations

Startup data-migration runner gated by `Config::schema_version`. Each migration is a one-shot, idempotent transformation of on-disk data (the persisted `config.toml` and session transcripts). The runner — `run_pending` — is invoked from `Config::load_or_init` and is a fast no-op for workspaces whose `schema_version` already matches `CURRENT_SCHEMA_VERSION`. Failures are logged but never block startup; the next launch retries from the same starting version.

> Not to be confused with the sibling `src/openhuman/migration/` (singular), which is a **user-triggered RPC** that imports memory from a legacy OpenClaw workspace. This module (`migrations`, plural) is the **automatic schema-version runner** that fires once per workspace on the first launch of a new build.

## Responsibilities

- Maintain `CURRENT_SCHEMA_VERSION` (currently `6`) as the target schema version, bumped alongside every new migration.
- Run pending migrations in order, each guarded by an exact `schema_version ==` check so a failed earlier step is never skipped (the 0→1 step uses `< 1`).
- Persist each version bump via `Config::save()` **only after** the migration reports success; on a save failure, roll the in-memory `schema_version` back so the next launch retries the same gate.
- Offload the blocking fs walk (`phase_out_profile_md`, 0→1) onto `tokio::task::spawn_blocking` to keep the executor responsive; pure in-memory config mutations run inline.
- Coalesce two independent, idempotent migrations behind the single 5→6 transition (`repair_http_request_limits` + `reconcile_orphaned_providers`); bump + save only when both succeed.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/migrations/mod.rs` | Module docstring + the `run_pending` runner and `CURRENT_SCHEMA_VERSION` constant; declares each migration as a private `mod`. |
| `src/openhuman/migrations/phase_out_profile_md.rs` | **0→1.** Deletes `{workspace}/PROFILE.md` and strips `### PROFILE.md` blocks from persisted JSONL session transcripts (`session_raw/**`) and `.md` companions (`sessions/**`). Blocking fs I/O. |
| `src/openhuman/migrations/unify_ai_provider_settings.rs` | **1→2.** Consolidates scattered AI-provider settings into per-workload provider strings; seeds `cloud_providers` (always an `Openhuman` entry) and migrates a non-backend `inference_url` into a `Custom` cloud entry. Pure in-memory. |
| `src/openhuman/migrations/retire_chat_v1_model.rs` | **2→3.** Remaps a persisted `default_model = "chat-v1"` to `"reasoning-quick-v1"` (backend dropped `chat-v1` from its strict registry; sub-agent spawns 400'd). Pure in-memory. |
| `src/openhuman/migrations/expand_autonomy_defaults.rs` | **3→4.** Additively merges expanded `autonomy.allowed_commands` / `auto_approve` defaults (PR #2500) and bumps `max_actions_per_hour` from the old `20` to `u32::MAX` only when still exactly `20`. Pure in-memory. |
| `src/openhuman/migrations/remove_write_auto_approve.rs` | **4→5.** Removes `file_write` / `edit_file` from `autonomy.auto_approve` so Supervised mode resumes its ask-before-edit prompt. Pure in-memory. |
| `src/openhuman/migrations/repair_http_request_limits.rs` | **5→6 (a).** Coerces stale-zero `[http_request]` `timeout_secs` / `max_response_size` to schema defaults (30s / 1 MB); a persisted `0` is an instant timeout / empty-body cap that serde defaults don't repair. Pure in-memory. |
| `src/openhuman/migrations/reconcile_orphaned_providers.rs` | **5→6 (b).** Resets per-workload `*_provider` strings (and a dangling `primary_cloud`) that point at a cloud provider no longer in `cloud_providers`, which the inference factory hard-errors on; mirrors the factory's exact grammar. Pure in-memory. |
| `src/openhuman/migrations/mod_tests.rs` | Tests for `run_pending` ordering, gating, rollback-on-save-failure. |
| `src/openhuman/migrations/*_tests.rs` | Per-migration unit tests (`phase_out_profile_md_tests.rs`, `reconcile_orphaned_providers_tests.rs`, `unify_ai_provider_settings_tests.rs`); other migrations keep inline `#[cfg(test)]` tests. |

## Public surface

- `migrations::run_pending(config: &mut Config) -> impl Future<Output = ()>` — the only public entry point; called by `Config::load_or_init`.
- `migrations::CURRENT_SCHEMA_VERSION: u32` — target schema version (`6`).

All individual migration modules (`phase_out_profile_md`, `unify_ai_provider_settings`, …) are **private** to the module; each exposes a `run(...)` fn and a `*Stats` struct used internally by the runner for diagnostics logging.

## RPC / controllers

None. This module exposes no controllers or RPC methods (the RPC-facing migration surface lives in the sibling `src/openhuman/migration/`, singular).

## Agent tools

None.

## Events

None — no event-bus publishers or subscribers.

## Persistence

Does not own a `store.rs`. Its effects are written through other layers:

- `Config` mutations (incl. `schema_version`) persisted via `Config::save()` to `config.toml`, driven by the runner in `mod.rs`.
- `phase_out_profile_md` rewrites session transcript files in place under `{workspace}/session_raw/**` and `{workspace}/sessions/**`, and deletes `{workspace}/PROFILE.md`.

## Dependencies

- `crate::openhuman::config` (`Config`, `HttpRequestConfig`, `schema::cloud_providers::{CloudProviderCreds, AuthStyle, CloudProviderType, generate_provider_id}`, `schema::{MODEL_CHAT_V1, MODEL_REASONING_QUICK_V1}`) — the migrations read and mutate the config struct; `schema_version` is the gating field.
- `crate::openhuman::agent::harness::session::transcript` (`transcript`, `SessionTranscript`, `write_transcript`) — `phase_out_profile_md` parses and rewrites persisted session transcripts byte-compatibly.
- `crate::openhuman::inference::provider::factory` — `reconcile_orphaned_providers` mirrors the factory's exact, case-sensitive provider-string grammar so "resolvable here" matches "resolvable at inference time"; `unify_ai_provider_settings` references the factory's provider-string format. (`inference::provider::ChatMessage` is imported only by tests.)

## Used by

- `src/openhuman/config/schema/load.rs` — calls `migrations::run_pending(&mut config)` from `Config::load_or_init` (three call sites) and seeds new configs with `schema_version: CURRENT_SCHEMA_VERSION`.
- `src/openhuman/mod.rs` — declares `pub mod migrations;`.

## Notes / gotchas

- **Best-effort, never blocks startup.** A migration error or a `spawn_blocking` join failure is logged at WARN and `run_pending` returns; the gate stays at the failing version and retries next launch.
- **Save-failure rollback.** Each step bumps `config.schema_version` in memory, then `Config::save().await`; if the save fails the version is rolled back to `previous_version` so disk and memory stay consistent.
- **Exact `==` gating (except 0→1).** Steps 1→2 through 4→5 guard on `schema_version == N`, not `< N+1`, so a failed earlier migration cannot be silently skipped. The first step uses `< 1`.
- **Idempotency is doubly guaranteed:** externally by the `schema_version` gate, and internally — every migration is a no-op when re-run on already-migrated data (e.g. additive merges guarded by `contains`, `is_none()` field checks, absence of a `### PROFILE.md` block).
- **5→6 shares one version bump for two modules.** Both `repair_http_request_limits` and `reconcile_orphaned_providers` run; the gate advances to 6 only if both succeed. Re-running the one that already succeeded on the next launch is a no-op.
- **Adding a migration:** add a `mod`, bump `CURRENT_SCHEMA_VERSION`, and add a `if config.schema_version == N` branch in `run_pending` that calls the module and bumps + saves on success (see the mod.rs docstring).
- Migration `run(...)` fns return `anyhow::Result<…>` to match the runner's dispatch signature even when the transform is currently infallible, so a future I/O-backed step slots in without churning the runner.

# scheduler_gate

Gates background AI work (memory-tree digests, embeddings, summarisation, triage, reflection, local inference) on live host conditions so the process doesn't make the machine visibly lag — especially on battery. It exposes a single process-wide decision point: background workers consult `current_policy()` for a cheap read, or `await wait_for_capacity()` to cooperatively block until the host is ready and hold a slot in a one-permit LLM semaphore. A background sampler refreshes host signals every 30s and recomputes the policy. A separate "signed out" override trumps everything to halt LLM work the moment the app session goes away.

## Responsibilities

- Sample host signals on a 30s cadence: power state (on AC / battery charge), recent global CPU usage, and deployment mode (server/container).
- Turn signals + user config into a `Policy` tier: `Aggressive` (server / always-on), `Normal` (desktop with headroom), `Throttled` (busy or on battery), `Paused` (user opted out, on battery with `require_ac_power`, CPU pressure, or signed out).
- Enforce a **process-wide single-slot LLM semaphore** so concurrent local-Ollama / bge-m3 calls can't saturate laptop RAM.
- Provide cooperative backoff: `wait_for_capacity()` resolves immediately in `Aggressive`/`Normal`, sleeps `throttled_backoff_ms` in `Throttled`, and re-polls every `paused_poll_ms` in `Paused` so callers resume the instant the gate flips back on.
- Expose a signed-out kill switch (`set_signed_out` / `is_signed_out`) consumed by the credentials lifecycle and 401-detection sites to stand down all background LLM work.
- Clamp out-of-domain config thresholds defensively so a malformed `config.toml` can't silently disable or force-throttle work.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/scheduler_gate/mod.rs` | Module docstring + re-exports of the public surface. |
| `src/openhuman/scheduler_gate/gate.rs` | Process-wide singleton: cached `State` (config + signals + policy), the 30s sampler task, the single-slot LLM semaphore, `LlmPermit` RAII guard, signed-out override, and `init_global`/`update_config`/`current_policy`/`current_signals`/`wait_for_capacity`. Holds the per-tokio-runtime test-state scaffolding. |
| `src/openhuman/scheduler_gate/policy.rs` | Pure decision logic: `decide(signals, cfg) -> Policy`, the `Policy` and `PauseReason` enums, and their `as_str` / `pause_reason` helpers. Evaluation order: user mode override → server mode → power-aware stand-down → hard CPU ceiling → battery/CPU headroom. |
| `src/openhuman/scheduler_gate/signals.rs` | `Signals` snapshot + `Signals::sample()`. Probes battery (via `starship_battery`), CPU usage (via `sysinfo`, two-refresh delta), and detects server/container mode. Honours `OPENHUMAN_ON_AC_POWER`, `OPENHUMAN_BATTERY_CHARGE`, `OPENHUMAN_DEPLOYMENT` env overrides plus Kubernetes / `/.dockerenv` heuristics. |

## Public surface

From `mod.rs`:

- **Functions** (`gate`): `init_global(&Config)`, `current_policy() -> Policy`, `current_signals() -> Signals`, `wait_for_capacity() -> Option<LlmPermit>`, `is_signed_out() -> bool`, `set_signed_out(bool)`.
- **Types**: `LlmPermit` (RAII semaphore guard, `#[must_use]`), `Policy` (`Aggressive` / `Normal` / `Throttled` / `Paused { reason }`), `PauseReason` (`UserDisabled` / `OnBattery` / `CpuPressure` / `SignedOut` / `Unknown`), `Signals`.
- Not re-exported but `pub` on `gate`: `update_config(SchedulerGateConfig)`.
- Test-only: `SignedOutTestGuard` (RAII flag snapshot/restore), `try_acquire_llm_permit`, `available_llm_permits`.

## RPC / controllers

None. This module exposes no JSON-RPC controllers, schemas, or `handle_*` functions — it is consulted in-process via direct function calls.

## Agent tools

None.

## Events

No `bus.rs` and no `DomainEvent` publish/subscribe of its own. It is invoked directly; the credentials domain (`credentials/bus.rs`, `credentials/ops.rs`) calls `set_signed_out` from session lifecycle handlers.

## Persistence

None. State is process-memory only (`OnceLock<Arc<RwLock<State>>>` + a process-wide `AtomicBool` signed-out flag and `OnceLock<Arc<Semaphore>>`). It does not persist to disk; the signed-out flag is reseated from the on-disk session at startup by `init_global` callers / credentials code, not stored here.

## Dependencies

- `crate::openhuman::config` — reads `Config`, `SchedulerGateConfig` (the `[scheduler_gate]` block: `mode`, `battery_floor`, `cpu_busy_threshold_pct`, `cpu_severe_pct`, `throttled_backoff_ms`, `paused_poll_ms`, `require_ac_power`) and `SchedulerGateMode` (`Auto` / `AlwaysOn` / `Off`).
- External crates: `parking_lot` (RwLock/Mutex), `tokio::sync::Semaphore`, `sysinfo` (CPU), `starship_battery` (power probe), `once_cell` (lazy CPU `System`).

No dependency on any other `openhuman` domain or on `crate::core::*`.

## Used by

Consumed in-process across the codebase (discoverable via `grep scheduler_gate`):

- **Background workers / pipelines**: `memory/schema.rs`, `memory_queue/worker.rs`, `memory_tree/tree/rpc.rs`, `memory_sync/composio/periodic.rs`, `subconscious/engine.rs`, `learning/reflection.rs`, `autocomplete/core/engine.rs`, `task_sources/route.rs`, `agent/task_dispatcher.rs`, `agent/triage/evaluator.rs`.
- **Inference layer**: `inference/provider/openhuman_backend.rs`, `inference/provider/factory.rs`, `inference/local/service/{vision_embed.rs,public_infer.rs}`, `inference/voice/postprocess.rs`.
- **Credentials lifecycle** (signed-out kill switch): `credentials/ops.rs`, `credentials/bus.rs`.
- **Bootstrap / transport**: `core/jsonrpc.rs` (calls `init_global` during server bootstrap), `core/observability.rs`, plus the domain wiring in `openhuman/mod.rs` and config schema in `config/schema/scheduler_gate.rs`.

## Notes / gotchas

- **Single LLM slot is deliberate** (`LLM_SLOTS = 1`): concurrent local Ollama / bge-m3 calls (~1.3 GB resident each) have crashed the user's laptop. Cloud-backend calls bypass this semaphore at the worker layer because they're bandwidth-bound, not RAM-bound.
- **Backoff happens before semaphore acquisition** so a `Paused`/`Throttled` mode doesn't pile tasks into the semaphore wait queue — they sit in the policy poll loop instead.
- **`current_policy()` defaults to `Normal` and `wait_for_capacity()` acquires directly when `STATE` is uninitialised** (unit tests / pre-`init_global` bootstrap) so callers never deadlock on a sampler that will never start.
- **Signed-out override is gated on `STATE.get().is_some()`** on both the reader (`current_policy`/`wait_for_capacity`) and writer (`set_signed_out`) sides. Without this, a stale per-test `signed_out=true` flag (from `clear_session` / 401 / `SessionExpiredSubscriber` tests) would make every later `wait_for_capacity` caller poll forever — the source of the post-#1516 triage-evaluator hangs.
- **Test isolation**: in `cfg(test)` the semaphore and signed-out flag are keyed per tokio runtime ID (`test_state`) so parallel cargo workers and libtest thread reuse don't leak state across `#[tokio::test]`s. `SignedOutTestGuard` snapshots/restores the flag and bypasses the writer-side `STATE` gate.
- **`init_global` is idempotent** (`std::sync::Once`); live config changes go through `update_config`, which recomputes the policy immediately.
- **Server-mode detection** never infers server from "no battery" alone (desktops have none); it requires Linux + no battery + no `DISPLAY`/`WAYLAND_DISPLAY`, or explicit env / k8s / docker signals.
- `PauseReason::OnBattery` and `CpuPressure` are the active power-aware (#1073) reasons; `Unknown` is a placeholder fallback.

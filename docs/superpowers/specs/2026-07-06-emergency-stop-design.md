# Emergency Stop for desktop automation — design

Issue: [tinyhumansai/openhuman#4255](https://github.com/tinyhumansai/openhuman/issues/4255) — "Desktop automation safety previews, confirmations, history, and emergency stop".

This spec covers **slice 1: Emergency Stop** only. The issue is an epic (previews, confirmations, history, emergency stop, backups, Windows support, reusable workflows); its acceptance criteria call for one slice landed end-to-end first. Emergency Stop is the safety-critical control and is self-contained.

## Goal

A prominent, always-available control that:

1. **Immediately halts** all running/queued desktop automation, and
2. **Blocks any further automated actions** until the user explicitly resumes.

It is a **fail-closed kill switch**: while engaged, every automated real-world action is refused.

## Scope decisions (approved)

- **Stop behavior:** set a global halt flag (blocks all further external-effect / accessibility actions fail-closed), stop the accessibility session, and cascade-deny all pending approvals. The running agent turn can take no further real-world actions. (We do **not** hard-abort in-flight chat turns in this slice — blocking every action chokepoint achieves the safety goal without touching the turn/cancel machinery.)
- **Persistence:** in-memory only; a restart clears the halt (reset on boot). Persisting a halt across restarts is a follow-up.
- **Backend (`backend-alphahuman`):** no changes. Desktop automation executes in the Tauri Rust core; the backend's execution-session flow is a separate (email/Telegram) subsystem. A server-side execution-session cancel is a follow-up, not this slice.

## Existing infrastructure this builds on

- `src/openhuman/approval/` — `ApprovalGate` parks/denies external-effect tool calls, keeps a SQLite audit trail, fail-closed 10-min TTL. We reuse its pending-list + deny path for cascade-deny.
- `src/openhuman/tinyagents/middleware.rs::wrap_tool` — every external-effect/dangerous tool call is already intercepted here (`has_external_effect`, `gate.intercept_audited`). This is our primary enforcement chokepoint.
- `src/openhuman/screen_intelligence/` — `ops.rs::accessibility_input_action` dispatches clicks/typing to `input.rs`, which already has a per-session `panic_stop` action and session stop. This is our second chokepoint + the session-stop reuse.
- `src/core/all.rs` controller registry + `src/openhuman/channels/providers/web/event_bus.rs` `ApprovalSurfaceSubscriber` — the pattern for RPC registration and bridging domain events to a web-channel socket event.

## Architecture

### New core domain — `src/openhuman/emergency_stop/`

Follows the canonical module shape (`mod.rs` export-only; `types.rs`; `state.rs`; `ops.rs`; `schemas.rs`).

- **`state.rs`** — process-global `EmergencyStop` in a `OnceCell`, holding `AtomicBool engaged` + `Mutex<Option<HaltInfo>>` (`reason: String`, `engaged_at_ms: u64`, `source: HaltSource`). Public: `global()` / `try_global()`, `is_engaged()`, `engage(info)`, `clear()`, `snapshot() -> HaltState`. Mirrors `ApprovalGate` global-singleton ergonomics. `try_global()` → `None` means "no switch installed" → treated as not-engaged (never blocks) so headless/CLI paths are unaffected.
- **`types.rs`** — `HaltState { engaged: bool, reason: Option<String>, engaged_at_ms: Option<u64>, source: Option<HaltSource> }` (serde); `HaltSource` enum (`User`, `Hotkey`, `System`).
- **`ops.rs`** — handlers returning `RpcOutcome<HaltState>`:
  - `emergency_stop(reason, source)` — engage flag; then best-effort: stop the accessibility session (reuse existing stop path) and cascade-deny all `ApprovalGate` pending rows (`list_pending` → `decide(deny)`); publish `AutomationHalted`; return snapshot. Idempotent (already-engaged is a no-op success).
  - `emergency_resume()` — clear flag; publish `AutomationResumed`; return snapshot. Idempotent.
  - `emergency_status()` — return snapshot.
- **`schemas.rs` + `mod.rs`** — controllers → RPC `openhuman.emergency_stop`, `openhuman.emergency_resume`, `openhuman.emergency_status`; registered in `src/core/all.rs`.
- **Events** — `DomainEvent::AutomationHalted { reason, source }` / `AutomationResumed` (add to `src/core/event_bus/events.rs`, extend `domain()` match → `system`). A subscriber in `web.rs` (or extending `ApprovalSurfaceSubscriber`) bridges them to a web-channel socket event (`automation_halt`) so all UI surfaces update live.
- **Install** — `EmergencyStop::init_global()` at core startup next to `ApprovalGate::init_global()` in `src/core/jsonrpc.rs`.

### Enforcement (the "block further actions" invariant) — fail-closed at two chokepoints

1. **`tinyagents/middleware.rs::wrap_tool`** — at the top of the external-effect/dangerous path, if `EmergencyStop::is_engaged()`, refuse the call before `execute()` with a clear `POLICY_DENIED_MARKER`-style "emergency stop engaged" reason. This stops the agent loop from taking further real-world actions. (**Scope note for this slice:** the refusal is surfaced via a `tracing::warn!` and the `AutomationHalted` domain event / `automation_halt` socket broadcast, but is **not** recorded through `ApprovalGate::intercept_audited` as an `Aborted` audit row. Writing halted refusals into the approval audit trail needs a new gate API and is tracked as a follow-up.)
2. **`screen_intelligence/ops.rs::accessibility_input_action`** — if engaged, short-circuit to `{ accepted: false, blocked: true, reason: "emergency_stop" }` (except the existing `panic_stop` action, which must still pass so a stop is never blocked by a stop).

Both checks are cheap (`AtomicBool` load) and fail-open only when no switch is installed.

### Frontend (`app/src/`)

- **Redux `safetySlice`** — `{ halted: bool, reason?: string, since?: number, source?: string }`; actions `setHalt`, `clearHalt`, `hydrateHalt`.
- **`services/api/emergencyApi.ts`** — `emergencyStop()`, `emergencyResume()`, `emergencyStatus()` via `core_rpc_relay` (`coreRpcClient`).
- **Socket handler** — subscribe to `automation_halt`; dispatch `setHalt`/`clearHalt`. Hydrate via `emergencyStatus()` on boot.
- **UI**
  - A persistent **Emergency Stop** button in the app shell / chat header (always visible), `data-analytics-id` for analytics.
  - When halted, a **banner** ("Automation halted — {reason}") with a **Resume** action.
  - All copy through `useT()`; keys added to `en.ts` and every locale file (CI enforces parity).

## Data flow

```
User clicks Emergency Stop
  → emergencyApi.emergencyStop()  (core_rpc_relay → openhuman.emergency_stop)
  → ops::emergency_stop: engage flag; stop a11y session; cascade-deny pending approvals
  → publish AutomationHalted → web subscriber → socket 'automation_halt'
  → all clients: safetySlice.setHalt → button shows halted state + banner

Agent tries another tool while halted
  → middleware.wrap_tool sees is_engaged() → deny (tracing warn + halt event; audit-row write deferred) → agent cannot act
Agent/vision tries accessibility_input_action while halted
  → ops sees is_engaged() → { accepted:false, blocked:true, reason:'emergency_stop' }

User clicks Resume
  → openhuman.emergency_resume → clear flag → AutomationResumed → socket → clearHalt
```

## Error handling

- **Fail-closed:** any uncertainty (switch installed and engaged) blocks. No installed switch (CLI/headless) never blocks.
- **Best-effort side effects on engage:** if stopping the a11y session or cascade-denying an approval errors, the halt flag is still set and the error is logged — the primary invariant (flag set → actions blocked) never depends on a side effect succeeding.
- **Idempotent** stop/resume so double-clicks and repeated socket events are safe.

## Testing (≥80% diff coverage — merge gate)

**Rust unit tests** (inline `#[cfg(test)]` / sibling `*_tests.rs`):
- `state`: engage/clear/snapshot; `is_engaged` transitions; `try_global` None → not engaged.
- `ops`: stop sets flag + emits `AutomationHalted`; resume clears + emits `AutomationResumed`; stop is idempotent; cascade-deny denies pending rows; best-effort side-effect failure still sets the flag.
- middleware chokepoint: external-effect tool refused while halted, allowed after resume.
- `accessibility_input_action`: blocked while halted; `panic_stop` still passes while halted.

**JSON-RPC E2E** (`tests/json_rpc_e2e.rs`): `emergency_status` (not halted) → `emergency_stop` → `emergency_status` (halted, reason) → `emergency_resume` → `emergency_status` (not halted).

**Vitest** (`app/src/**`): `safetySlice` reducers; `emergencyApi` calls correct RPC methods; Emergency Stop button dispatches stop and reflects halted state; banner renders + Resume dispatches resume; socket handler maps events to store.

## Out of scope (follow-ups tracked against #4255)

- Action previews, backup-before-overwrite, activity-history UI, reusable app workflows, Windows-specific assessment.
- Persisting halt across restarts; hard-aborting in-flight chat turns; server-side (`backend-alphahuman`) execution-session cancel.
- A global OS panic **hotkey** binding for emergency stop (the per-session `panic_stop` exists; a global hotkey is a follow-up).

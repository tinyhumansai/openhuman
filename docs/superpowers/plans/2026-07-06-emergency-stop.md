# Emergency Stop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A fail-closed "Emergency Stop" kill switch that instantly halts all running/queued desktop automation and blocks any further automated action until the user explicitly resumes.

**Architecture:** A new process-global `EmergencyStop` singleton in the Rust core (`src/openhuman/emergency_stop/`), mirroring the `ApprovalGate` `OnceLock` pattern. Three RPCs (`openhuman.emergency_stop|resume|status`) engage/clear/read it. Two fail-closed enforcement chokepoints consult it: the tinyagents approval middleware (blocks external-effect tool calls) and `accessibility_input_action` (blocks clicks/typing). Engaging also stops the accessibility session and cascade-denies pending approvals. The React app gets a persistent Emergency Stop button + halted banner backed by a Redux `safetySlice`, driven by the RPC responses and boot-time hydration.

**Tech Stack:** Rust (tokio, serde, `anyhow`), JSON-RPC controller registry, React 19 + TypeScript + Redux Toolkit + Vitest, i18n via `useT()`.

**Spec:** `docs/superpowers/specs/2026-07-06-emergency-stop-design.md`

## Global Constraints

- **Never write code on `main`.** Work is on branch `feat/desktop-safety-4255` (already created).
- **Fail-closed:** when the switch is installed AND engaged, block. When no switch is installed (`try_global()` → `None`, e.g. CLI/headless), never block.
- **Diff coverage ≥ 80% on changed lines** (merge gate: `frontend-coverage`/`rust-core-coverage`).
- **Rust module shape** (AGENTS.md): `mod.rs` export-only; `types.rs` serde types; `state.rs` state; `ops.rs` logic returning `RpcOutcome<T>`; `schemas.rs` controllers. New functionality → dedicated subdirectory; no new root-level `*.rs`.
- **RPC naming:** `openhuman.<namespace>_<function>` — here namespace `emergency`, functions `stop`/`resume`/`status`.
- **Controller exposure:** register via `src/core/all.rs` registry, not branches in `cli.rs`/`jsonrpc.rs`.
- **i18n:** all UI text through `useT()`; add keys to `app/src/lib/i18n/en.ts` **and** real translations in every sibling locale file (`app/src/lib/i18n/<locale>.ts` for `ar, bn, de, es, fr, hi, id, it, ko, pl, pt, ru, zh-CN`). CI enforces parity (`pnpm i18n:check`).
- **Debug logging:** grep-friendly prefixes (`[emergency]`, `[rpc:emergency_*]`); log entry/exit, state transitions, errors; never log secrets/PII.
- **Frontend:** no dynamic imports in `app/src`; use `invoke('core_rpc_relay', …)` via `coreRpcClient`; guard Tauri with `isTauri()`/try-catch.
- **Rust checks:** `cargo check --manifest-path Cargo.toml` (add `GGML_NATIVE=OFF` on macOS Apple Silicon). Tests: `pnpm test:rust` or `bash scripts/test-rust-with-mock.sh --test <name>`; targeted lib tests: `cargo test --manifest-path Cargo.toml <filter>`.
- **Frontend checks:** `pnpm typecheck`, `pnpm lint`, `pnpm test`.

---

## File Structure

**Rust core (new domain `src/openhuman/emergency_stop/`):**
- `mod.rs` — module docstring, `pub mod` decls, `pub use` re-exports, controller-schema pair.
- `types.rs` — `HaltState`, `HaltSource` (serde).
- `state.rs` — `EmergencyStop` global singleton (`OnceLock`), `init_global`/`try_global`/`is_engaged`/`engage`/`clear`/`snapshot`.
- `ops.rs` — `emergency_stop`/`emergency_resume`/`emergency_status` returning `RpcOutcome<HaltState>`; cascade-deny + a11y stop; publishes events.
- `schemas.rs` — controller schemas + `handle_*` fns.

**Rust core (modified):**
- `src/core/event_bus/events.rs` — add `AutomationHalted`/`AutomationResumed` variants + `domain()` + `name()` arms.
- `src/core/all.rs` — register emergency controllers.
- `src/core/jsonrpc.rs` — install `EmergencyStop::init_global()` at boot; register socket bridge subscriber.
- `src/openhuman/tinyagents/middleware.rs` — halt check in `ApprovalSecurityMiddleware::wrap_tool`.
- `src/openhuman/screen_intelligence/ops.rs` — halt check in `accessibility_input_action`.
- `src/openhuman/channels/providers/web/event_bus.rs` — `AutomationHaltSubscriber` bridging events → `automation_halt` socket event.
- `tests/json_rpc_e2e.rs` — stop→status→resume E2E.

**Frontend (new):**
- `app/src/store/safetySlice.ts` (+ `safetySlice.test.ts`) — halted state.
- `app/src/services/api/emergencyApi.ts` (+ `emergencyApi.test.ts`) — RPC client.
- `app/src/components/safety/EmergencyStopButton.tsx` (+ test) — button.
- `app/src/components/safety/AutomationHaltedBanner.tsx` (+ test) — banner + Resume.

**Frontend (modified):**
- `app/src/store/index.ts` (or root reducer) — mount `safety` reducer.
- `app/src/services/socketService.ts` — handle `automation_halt` socket event.
- app shell/header (e.g. `app/src/components/layout/*` or `Conversations` header) — mount button + banner + boot hydration.
- `app/src/lib/i18n/locales/*.ts` — i18n keys.

**Note on exact neighboring types:** three field-shapes are already confirmed from the codebase — `InputActionResult { accepted: bool, blocked: bool, reason: Option<String> }` (`screen_intelligence/types.rs:144`), `InputActionParams { action: String, .. }` (`types.rs:133`), and the `WebChannelEvent` bridge pattern (`web/event_bus.rs`). Before Task 12's socket bridge, read `src/core/socketio` for the exact `WebChannelEvent` fields (the artifact/approval bridges set `event`, `client_id`, `thread_id`, `args`, `..Default::default()`).

---

## Task 1: Event variants — `AutomationHalted` / `AutomationResumed`

**Files:**
- Modify: `src/core/event_bus/events.rs` (add variants near the System lifecycle group ~line 1025; extend `domain()` ~1283 and `name()` ~1540)

**Interfaces:**
- Produces: `DomainEvent::AutomationHalted { reason: Option<String>, source: String }`, `DomainEvent::AutomationResumed { source: String }`. Both map to domain `"system"`.

- [ ] **Step 1: Add the two variants.** In the `DomainEvent` enum, in the System-lifecycle region, add:

```rust
    /// Emergency stop engaged — all desktop automation is halted and every
    /// external-effect / accessibility action is refused until resumed.
    /// Published by `emergency_stop::ops::emergency_stop`; bridged to the
    /// `automation_halt` web-channel socket event.
    AutomationHalted {
        /// Optional human-readable reason (redacted of PII by the caller).
        reason: Option<String>,
        /// Who engaged it: `"user"`, `"hotkey"`, or `"system"`.
        source: String,
    },
    /// Emergency stop cleared — automation may resume. Published by
    /// `emergency_stop::ops::emergency_resume`.
    AutomationResumed {
        /// Who cleared it: `"user"`, `"hotkey"`, or `"system"`.
        source: String,
    },
```

- [ ] **Step 2: Extend `domain()`.** In the `pub fn domain(&self)` match, add to the `"system"` arm (alongside `HarnessInitCompleted`):

```rust
            | Self::AutomationHalted { .. }
            | Self::AutomationResumed { .. } => "system",
```

- [ ] **Step 3: Extend `name()`.** In the `name()` match (near the `ApprovalRequested => "ApprovalRequested"` arms):

```rust
            Self::AutomationHalted { .. } => "AutomationHalted",
            Self::AutomationResumed { .. } => "AutomationResumed",
```

- [ ] **Step 4: Add a unit test.** Append to the `#[cfg(test)]` module in `events.rs` (or create one if none — match the file's existing test style):

```rust
    #[test]
    fn automation_events_map_to_system_domain() {
        let halted = DomainEvent::AutomationHalted { reason: Some("user".into()), source: "user".into() };
        let resumed = DomainEvent::AutomationResumed { source: "user".into() };
        assert_eq!(halted.domain(), "system");
        assert_eq!(resumed.domain(), "system");
        assert_eq!(halted.name(), "AutomationHalted");
        assert_eq!(resumed.name(), "AutomationResumed");
    }
```

- [ ] **Step 5: Compile + test.**

Run: `GGML_NATIVE=OFF cargo test --manifest-path Cargo.toml -p openhuman automation_events_map_to_system_domain`
Expected: PASS (build succeeds, 1 test passes). If `name()`/`domain()` have exhaustive-match compile errors, fix the arms until it builds.

- [ ] **Step 6: Commit.**

```bash
git add src/core/event_bus/events.rs
git commit -m "feat(events): add AutomationHalted/AutomationResumed domain events (#4255)"
```

---

## Task 2: `emergency_stop` types

**Files:**
- Create: `src/openhuman/emergency_stop/types.rs`

**Interfaces:**
- Produces: `HaltState { engaged: bool, reason: Option<String>, engaged_at_ms: Option<u64>, source: Option<String> }` (serde, `Clone`, `Debug`, `PartialEq`, `Default`). Used by `state.rs`, `ops.rs`, `schemas.rs`.

- [ ] **Step 1: Write the failing test.** Create `src/openhuman/emergency_stop/types.rs`:

```rust
//! Serde domain types for the emergency-stop kill switch.

use serde::{Deserialize, Serialize};

/// Snapshot of the emergency-stop switch, returned by every emergency RPC and
/// surfaced in the UI. `engaged == false` is the resting state.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HaltState {
    /// Whether automation is currently halted.
    pub engaged: bool,
    /// Human-readable reason for the halt (redacted of PII), when engaged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Unix-epoch milliseconds when the halt was engaged, when engaged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engaged_at_ms: Option<u64>,
    /// Who engaged it: `"user"`, `"hotkey"`, or `"system"`, when engaged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_halt_state_is_not_engaged() {
        let s = HaltState::default();
        assert!(!s.engaged);
        assert!(s.reason.is_none());
        assert!(s.engaged_at_ms.is_none());
    }

    #[test]
    fn resting_state_serializes_to_engaged_false_only() {
        let json = serde_json::to_string(&HaltState::default()).unwrap();
        assert_eq!(json, r#"{"engaged":false}"#);
    }

    #[test]
    fn engaged_state_roundtrips() {
        let s = HaltState { engaged: true, reason: Some("user".into()), engaged_at_ms: Some(42), source: Some("user".into()) };
        let back: HaltState = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(s, back);
    }
}
```

- [ ] **Step 2: Run to verify it fails to build.** (Module not declared yet — see Task 4 wires `mod.rs`; for now this task's test runs once `mod.rs` exists. To keep TDD honest, do Task 3 & the `mod.rs` skeleton, then run.) Run after Task 4: `GGML_NATIVE=OFF cargo test --manifest-path Cargo.toml -p openhuman emergency_stop::types`
Expected (before impl wired): FAIL to compile ("file not found for module" / unresolved).

- [ ] **Step 3: (impl already written in Step 1).**

- [ ] **Step 4: Run after `mod.rs` exists (Task 4).** Expected: 3 tests PASS.

- [ ] **Step 5: Commit** (batched with Task 3–4, since the module must be wired to compile).

---

## Task 3: `emergency_stop` state (global singleton)

**Files:**
- Create: `src/openhuman/emergency_stop/state.rs`

**Interfaces:**
- Consumes: `HaltState` (Task 2).
- Produces: `EmergencyStop` with associated fns `init_global() -> Arc<EmergencyStop>`, `try_global() -> Option<Arc<EmergencyStop>>`, and methods `is_engaged(&self) -> bool`, `engage(&self, reason: Option<String>, source: &str, now_ms: u64)`, `clear(&self)`, `snapshot(&self) -> HaltState`. Free fn `is_engaged_global() -> bool` (false when no switch installed).

- [ ] **Step 1: Write state + tests.** Create `src/openhuman/emergency_stop/state.rs`:

```rust
//! Process-global emergency-stop switch. Mirrors the `ApprovalGate`
//! `OnceLock` install pattern: `init_global` is idempotent, `try_global`
//! returns `None` when never installed (CLI/headless → never blocks).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use super::types::HaltState;

static GLOBAL_STOP: OnceLock<Arc<EmergencyStop>> = OnceLock::new();

#[derive(Debug)]
struct HaltInfo {
    reason: Option<String>,
    engaged_at_ms: u64,
    source: String,
}

/// Coordinator for the emergency-stop kill switch.
#[derive(Debug)]
pub struct EmergencyStop {
    engaged: AtomicBool,
    info: Mutex<Option<HaltInfo>>,
}

impl EmergencyStop {
    /// Install the process-global switch. Idempotent — re-install returns the
    /// existing switch so repeated boots in tests don't panic.
    pub fn init_global() -> Arc<EmergencyStop> {
        if let Some(existing) = GLOBAL_STOP.get() {
            return existing.clone();
        }
        let stop = Arc::new(EmergencyStop { engaged: AtomicBool::new(false), info: Mutex::new(None) });
        let _ = GLOBAL_STOP.set(stop.clone());
        GLOBAL_STOP.get().cloned().unwrap_or(stop)
    }

    /// The global switch when installed; `None` means "no switch" → callers
    /// treat as not-engaged (never block).
    pub fn try_global() -> Option<Arc<EmergencyStop>> {
        GLOBAL_STOP.get().cloned()
    }

    /// Whether automation is currently halted.
    pub fn is_engaged(&self) -> bool {
        self.engaged.load(Ordering::SeqCst)
    }

    /// Engage the halt. Idempotent — re-engaging refreshes reason/source/time.
    pub fn engage(&self, reason: Option<String>, source: &str, now_ms: u64) {
        {
            let mut guard = self.info.lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some(HaltInfo { reason, engaged_at_ms: now_ms, source: source.to_string() });
        }
        self.engaged.store(true, Ordering::SeqCst);
    }

    /// Clear the halt. Idempotent.
    pub fn clear(&self) {
        self.engaged.store(false, Ordering::SeqCst);
        let mut guard = self.info.lock().unwrap_or_else(|e| e.into_inner());
        *guard = None;
    }

    /// Current snapshot for RPC/UI.
    pub fn snapshot(&self) -> HaltState {
        if !self.is_engaged() {
            return HaltState::default();
        }
        let guard = self.info.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(info) => HaltState {
                engaged: true,
                reason: info.reason.clone(),
                engaged_at_ms: Some(info.engaged_at_ms),
                source: Some(info.source.clone()),
            },
            None => HaltState { engaged: true, ..Default::default() },
        }
    }
}

/// Global convenience: is a switch installed AND engaged? False when no
/// switch is installed (CLI/headless) so those paths are never blocked.
pub fn is_engaged_global() -> bool {
    EmergencyStop::try_global().map(|s| s.is_engaged()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engage_then_snapshot_reports_engaged() {
        let stop = EmergencyStop { engaged: AtomicBool::new(false), info: Mutex::new(None) };
        assert!(!stop.is_engaged());
        stop.engage(Some("user".into()), "user", 1234);
        assert!(stop.is_engaged());
        let snap = stop.snapshot();
        assert!(snap.engaged);
        assert_eq!(snap.reason.as_deref(), Some("user"));
        assert_eq!(snap.engaged_at_ms, Some(1234));
        assert_eq!(snap.source.as_deref(), Some("user"));
    }

    #[test]
    fn clear_resets_to_default_snapshot() {
        let stop = EmergencyStop { engaged: AtomicBool::new(false), info: Mutex::new(None) };
        stop.engage(None, "hotkey", 1);
        stop.clear();
        assert!(!stop.is_engaged());
        assert_eq!(stop.snapshot(), HaltState::default());
    }

    #[test]
    fn engage_is_idempotent_and_refreshes() {
        let stop = EmergencyStop { engaged: AtomicBool::new(false), info: Mutex::new(None) };
        stop.engage(Some("a".into()), "user", 1);
        stop.engage(Some("b".into()), "system", 2);
        assert!(stop.is_engaged());
        assert_eq!(stop.snapshot().reason.as_deref(), Some("b"));
        assert_eq!(stop.snapshot().source.as_deref(), Some("system"));
    }
}
```

- [ ] **Step 2 & 3:** impl is in Step 1.
- [ ] **Step 4: Run after `mod.rs` (Task 4).** Expected: 3 tests PASS.
- [ ] **Step 5: Commit** (batched with Task 4).

---

## Task 4: `emergency_stop` mod.rs + wire the module tree (makes Tasks 2–3 compile)

**Files:**
- Create: `src/openhuman/emergency_stop/mod.rs`
- Modify: `src/openhuman/mod.rs` (add `pub mod emergency_stop;` in the domain list, alphabetically near `embeddings`/`encryption`)

**Interfaces:**
- Produces: `pub use` of `EmergencyStop`, `is_engaged_global`, `HaltState`; and the controller-schema pair `all_emergency_controller_schemas`, `all_emergency_registered_controllers` (defined in Task 6's `schemas.rs`).

- [ ] **Step 1: Create `mod.rs`** (schemas referenced here are added in Task 6; declare the module now, add the re-exports in Task 6):

```rust
//! Emergency stop — a fail-closed kill switch for desktop automation.
//!
//! `EmergencyStop` is a process-global switch (mirrors `ApprovalGate`). When
//! engaged, the tinyagents approval middleware refuses external-effect tool
//! calls and `accessibility_input_action` refuses clicks/typing, until the
//! user resumes. Engaging also stops the accessibility session and
//! cascade-denies pending approvals. In-memory only (resets on restart).

pub mod ops;
pub mod schemas;
pub mod state;
pub mod types;

pub use schemas::{all_emergency_controller_schemas, all_emergency_registered_controllers};
pub use state::{is_engaged_global, EmergencyStop};
pub use types::HaltState;
```

- [ ] **Step 2: Register the domain module.** In `src/openhuman/mod.rs`, add (alphabetical):

```rust
pub mod emergency_stop;
```

- [ ] **Step 3: Build.** After Task 5 (`ops.rs`) and Task 6 (`schemas.rs`) exist, run:

Run: `GGML_NATIVE=OFF cargo test --manifest-path Cargo.toml -p openhuman emergency_stop::`
Expected: all `types`, `state` tests PASS (ops/schemas tests added in their tasks).

- [ ] **Step 4: Commit** (batched: types + state + mod once ops/schemas compile).

```bash
git add src/openhuman/emergency_stop/ src/openhuman/mod.rs
git commit -m "feat(emergency): halt-state types + global switch singleton (#4255)"
```

---

## Task 5: `emergency_stop` ops (engage/resume/status + side effects + events)

**Files:**
- Create: `src/openhuman/emergency_stop/ops.rs`

**Interfaces:**
- Consumes: `EmergencyStop` (Task 3), `HaltState` (Task 2), `DomainEvent::AutomationHalted/Resumed` (Task 1), `ApprovalGate` (`list_pending`, `decide`), `screen_intelligence::global_engine().disable(reason)`.
- Produces: `pub async fn emergency_stop(reason: Option<String>, source: &str) -> RpcOutcome<HaltState>`; `pub async fn emergency_resume(source: &str) -> RpcOutcome<HaltState>`; `pub async fn emergency_status() -> RpcOutcome<HaltState>`.

- [ ] **Step 1: Write ops + tests.** Create `src/openhuman/emergency_stop/ops.rs`:

```rust
//! Emergency-stop RPC operations: engage / resume / read the switch, plus the
//! best-effort side effects (stop the a11y session, cascade-deny pending
//! approvals) and event publication.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::rpc::RpcOutcome;

use super::state::EmergencyStop;
use super::types::HaltState;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Engage the kill switch: set the flag, then best-effort stop the a11y
/// session and cascade-deny pending approvals, then publish `AutomationHalted`.
/// Idempotent. Side-effect failures are logged but never fail the RPC — the
/// primary invariant (flag set → actions blocked) does not depend on them.
pub async fn emergency_stop(reason: Option<String>, source: &str) -> RpcOutcome<HaltState> {
    tracing::warn!(source, reason = ?reason, "[rpc:emergency_stop] entry — engaging kill switch");
    let stop = EmergencyStop::init_global();
    stop.engage(reason.clone(), source, now_ms());

    // Best-effort: stop the accessibility session so any in-flight click/type loop halts.
    // CONFIRMED: `engine.disable(reason: Option<String>) -> SessionStatus` (engine.rs:150);
    // `SessionStatus.active: bool` (types.rs:15).
    let a11y = crate::openhuman::screen_intelligence::global_engine()
        .disable(Some("emergency_stop".to_string()))
        .await;
    tracing::info!(active = a11y.active, "[emergency] accessibility session stopped");

    // Best-effort: cascade-deny every pending approval so parked tool calls fail closed.
    let denied = cascade_deny_pending();
    tracing::info!(denied, "[emergency] cascade-denied pending approvals");

    publish_global(DomainEvent::AutomationHalted { reason, source: source.to_string() });

    let snap = stop.snapshot();
    RpcOutcome::single_log(snap, format!("[emergency] halted (source={source}, denied={denied})"))
}

/// Deny all pending approvals. Returns how many were denied. Best-effort:
/// a per-row error is logged and skipped.
fn cascade_deny_pending() -> usize {
    use crate::openhuman::approval::{ApprovalDecision, ApprovalGate};
    let Some(gate) = ApprovalGate::try_global() else { return 0 };
    let rows = match gate.list_pending() {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(error = %err, "[emergency] list_pending failed during cascade-deny");
            return 0;
        }
    };
    let mut denied = 0;
    for row in rows {
        match gate.decide(&row.request_id, ApprovalDecision::Deny) {
            Ok(_) => denied += 1,
            Err(err) => tracing::warn!(request_id = %row.request_id, error = %err, "[emergency] deny failed"),
        }
    }
    denied
}

/// Clear the kill switch and publish `AutomationResumed`. Idempotent.
pub async fn emergency_resume(source: &str) -> RpcOutcome<HaltState> {
    tracing::info!(source, "[rpc:emergency_resume] entry — clearing kill switch");
    let stop = EmergencyStop::init_global();
    stop.clear();
    publish_global(DomainEvent::AutomationResumed { source: source.to_string() });
    RpcOutcome::single_log(stop.snapshot(), format!("[emergency] resumed (source={source})"))
}

/// Read the current switch state.
pub async fn emergency_status() -> RpcOutcome<HaltState> {
    let snap = EmergencyStop::try_global().map(|s| s.snapshot()).unwrap_or_default();
    tracing::debug!(engaged = snap.engaged, "[rpc:emergency_status] exit");
    RpcOutcome::new(snap, vec![])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stop_sets_flag_and_status_reports_engaged() {
        let out = emergency_stop(Some("user".into()), "user").await;
        assert!(out.value.engaged);
        let status = emergency_status().await;
        assert!(status.value.engaged);
        assert_eq!(status.value.source.as_deref(), Some("user"));
        // reset for other tests sharing the process-global switch
        let _ = emergency_resume("user").await;
    }

    #[tokio::test]
    async fn resume_clears_flag() {
        let _ = emergency_stop(None, "user").await;
        let out = emergency_resume("user").await;
        assert!(!out.value.engaged);
        assert!(!emergency_status().await.value.engaged);
    }

    #[tokio::test]
    async fn stop_is_idempotent() {
        let _ = emergency_stop(Some("a".into()), "user").await;
        let out = emergency_stop(Some("b".into()), "system").await;
        assert!(out.value.engaged);
        assert_eq!(out.value.reason.as_deref(), Some("b"));
        let _ = emergency_resume("user").await;
    }
}
```

Notes for the implementer:
- Confirm `RpcOutcome` exposes `.value` (the tests read `out.value`). If the field is named differently, read `src/rpc/*` for `RpcOutcome`'s public shape and adjust the test accessors (the `ops.rs` code uses only the constructors `RpcOutcome::new` / `RpcOutcome::single_log`, already used across the codebase).
- Confirm `ApprovalGate`, `ApprovalDecision` are re-exported from `crate::openhuman::approval` (README lists both under "Public surface"). `ApprovalDecision::Deny` is the deny variant.
- Confirm `global_engine().disable(reason)` returns a `SessionStatus` with an `active` field (seen in `ops.rs:121` `accessibility_stop_session`).

- [ ] **Step 2: Run to verify it fails first (before ops wired into mod).** After `mod.rs` includes `pub mod ops;` (Task 4), run:

Run: `GGML_NATIVE=OFF cargo test --manifest-path Cargo.toml -p openhuman emergency_stop::ops`
Expected: FAIL first if any signature mismatch; iterate until it compiles.

- [ ] **Step 3: Fix compile issues** (RpcOutcome field/accessor names, imports) until green.

- [ ] **Step 4: Run tests.** Expected: 3 ops tests PASS.

- [ ] **Step 5: Commit.**

```bash
git add src/openhuman/emergency_stop/ops.rs
git commit -m "feat(emergency): engage/resume/status ops with cascade-deny + a11y stop (#4255)"
```

---

## Task 6: `emergency_stop` schemas + registry wiring + boot install

**Files:**
- Create: `src/openhuman/emergency_stop/schemas.rs`
- Modify: `src/openhuman/emergency_stop/mod.rs` (re-export already added in Task 4)
- Modify: `src/core/all.rs` (register controllers, near approval ~line 160)
- Modify: `src/core/jsonrpc.rs` (install `EmergencyStop::init_global()` next to `ApprovalGate::init_global` ~line 2672)

**Interfaces:**
- Consumes: `ops` (Task 5), `HaltState` (Task 2).
- Produces: RPCs `emergency.stop`, `emergency.resume`, `emergency.status` (dispatched as `openhuman.emergency_stop|resume|status`); `all_emergency_controller_schemas()`, `all_emergency_registered_controllers()`.

- [ ] **Step 1: Write `schemas.rs`** (mirrors `approval/schemas.rs`):

```rust
//! Controller schemas + handlers for the `emergency` namespace.
//! Wires `emergency_stop`, `emergency_resume`, `emergency_status` into the
//! global registry consumed by `src/core/all.rs`.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::ops;

pub fn all_emergency_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("stop"), schemas("resume"), schemas("status")]
}

pub fn all_emergency_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController { schema: schemas("stop"), handler: handle_stop },
        RegisteredController { schema: schemas("resume"), handler: handle_resume },
        RegisteredController { schema: schemas("status"), handler: handle_status },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "stop" => ControllerSchema {
            namespace: "emergency",
            function: "stop",
            description: "Engage the emergency stop: halt all desktop automation and block further actions until resumed.",
            inputs: vec![FieldSchema {
                name: "reason",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Optional human-readable reason for the halt.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "state",
                ty: TypeSchema::Ref("HaltState"),
                comment: "Switch snapshot after engaging.",
                required: true,
            }],
        },
        "resume" => ControllerSchema {
            namespace: "emergency",
            function: "resume",
            description: "Clear the emergency stop so automation may resume.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "state",
                ty: TypeSchema::Ref("HaltState"),
                comment: "Switch snapshot after clearing.",
                required: true,
            }],
        },
        "status" => ControllerSchema {
            namespace: "emergency",
            function: "status",
            description: "Read the current emergency-stop switch state.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "state",
                ty: TypeSchema::Ref("HaltState"),
                comment: "Current switch snapshot.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "emergency",
            function: "unknown",
            description: "Unknown emergency function.",
            inputs: vec![],
            outputs: vec![FieldSchema { name: "error", ty: TypeSchema::String, comment: "Schema not defined.", required: true }],
        },
    }
}

fn handle_stop(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let reason = match params.get("reason") {
            Some(Value::String(s)) => Some(s.clone()),
            _ => None,
        };
        Ok(serde_json::to_value(ops::emergency_stop(reason, "user").await.value).map_err(|e| e.to_string())?)
    })
}

fn handle_resume(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        Ok(serde_json::to_value(ops::emergency_resume("user").await.value).map_err(|e| e.to_string())?)
    })
}

fn handle_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        Ok(serde_json::to_value(ops::emergency_status().await.value).map_err(|e| e.to_string())?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_controllers_match_schemas() {
        let c = all_emergency_registered_controllers();
        assert_eq!(c.len(), 3);
        let names: Vec<_> = c.iter().map(|c| c.schema.function).collect();
        assert_eq!(names, vec!["stop", "resume", "status"]);
    }

    #[test]
    fn stop_schema_has_optional_reason() {
        let s = schemas("stop");
        assert_eq!(s.namespace, "emergency");
        assert_eq!(s.inputs[0].name, "reason");
        assert!(!s.inputs[0].required);
    }
}
```

Notes for the implementer:
- The handler `to_json` pattern in `approval/schemas.rs` uses `outcome.into_cli_compatible_json()`. Prefer that exact helper for consistency: replace the `serde_json::to_value(...await.value)` lines with the approval pattern — call the op, then `outcome.into_cli_compatible_json()`. Read `approval/schemas.rs:201` (`to_json`) and copy it verbatim into this file, then `handle_* = to_json(ops::…().await)`. Adjust to whichever `RpcOutcome` serialization the registry expects (match approval exactly).
- `ControllerSchema`/`FieldSchema`/`TypeSchema`/`RegisteredController`/`ControllerFuture` imports mirror `approval/schemas.rs` lines 6–13.

- [ ] **Step 2: Register in `src/core/all.rs`.** Next to the approval registration (`controllers.extend(crate::openhuman::approval::all_approval_registered_controllers());`), add:

```rust
    controllers.extend(crate::openhuman::emergency_stop::all_emergency_registered_controllers());
```

Also add the schema list wherever approval's `all_controller_schemas` is aggregated (search `all_approval_registered_controllers`/`all_controller_schemas` usage in `all.rs` and mirror both).

- [ ] **Step 3: Install at boot in `src/core/jsonrpc.rs`.** Next to `ApprovalGate::init_global(cfg.clone(), session_id.clone());` (~line 2672), add:

```rust
            crate::openhuman::emergency_stop::EmergencyStop::init_global();
```

- [ ] **Step 4: Build + run schema tests.**

Run: `GGML_NATIVE=OFF cargo test --manifest-path Cargo.toml -p openhuman emergency_stop::schemas`
Expected: 2 tests PASS; whole crate compiles.

- [ ] **Step 5: Commit.**

```bash
git add src/openhuman/emergency_stop/schemas.rs src/openhuman/emergency_stop/mod.rs src/core/all.rs src/core/jsonrpc.rs
git commit -m "feat(emergency): RPC controllers (stop/resume/status) + boot install (#4255)"
```

---

## Task 7: Enforcement chokepoint 1 — approval middleware blocks while halted

**Files:**
- Modify: `src/openhuman/tinyagents/middleware.rs` (`ApprovalSecurityMiddleware::wrap_tool`, ~line 934, inside the `if self.has_external_effect(...)` block, before `gate.intercept_audited`)

**Interfaces:**
- Consumes: `crate::openhuman::emergency_stop::is_engaged_global`.

- [ ] **Step 1: Add the halt check.** In `wrap_tool`, immediately inside `if self.has_external_effect(&call.name, &call.arguments) {`, before the `if let Some(gate) = ApprovalGate::try_global()` line, insert:

```rust
            // Emergency stop: refuse every external-effect tool while halted,
            // before touching the approval gate. Fail-closed.
            if crate::openhuman::emergency_stop::is_engaged_global() {
                let reason = "Emergency stop is engaged — this action is blocked until you resume automation.".to_string();
                tracing::warn!(tool = %call.name, "[tinyagents::mw] emergency stop engaged — refusing tool call");
                return Ok(MiddlewareToolOutcome::Result(TaToolResult {
                    call_id: call.id,
                    name: call.name,
                    content: reason.clone(),
                    raw: None,
                    error: Some(reason),
                    elapsed_ms: 0,
                }));
            }
```

> **Audit-trail note (deferred):** the halt short-circuit returns immediately, so a refused call is NOT recorded through `ApprovalGate::intercept_audited` (which is what writes the "aborted" row for a denied external-effect call). This is a conscious scope choice for this slice — writing an `aborted` audit row from the middleware needs a new gate API (there is no such surface today), and the halted refusal is already surfaced via the `tracing::warn!` above plus the `AutomationHalted` domain event / `automation_halt` socket broadcast. Recording halted refusals in the approval audit trail is tracked as a follow-up; adjust either this step or the design spec's "audit trail" requirement once that follow-up lands.

- [ ] **Step 2: Write a unit test.** In the `#[cfg(test)]` module of `middleware.rs` (or a sibling `middleware_tests.rs` if one exists — match the file's convention), add a test that engages the global switch and asserts a halted external-effect call short-circuits. If constructing a full `RunContext`/`ToolHandler` is heavy, instead add a focused test in `emergency_stop` that exercises `is_engaged_global()` transitions and document the middleware behavior via an integration assertion in Task 10's E2E. Minimum viable unit test (pure guard behavior):

```rust
    #[test]
    fn emergency_guard_blocks_when_engaged() {
        use crate::openhuman::emergency_stop::EmergencyStop;
        let stop = EmergencyStop::init_global();
        stop.clear();
        assert!(!crate::openhuman::emergency_stop::is_engaged_global());
        stop.engage(Some("test".into()), "user", 0);
        assert!(crate::openhuman::emergency_stop::is_engaged_global());
        stop.clear();
    }
```

- [ ] **Step 3: Build + test.**

Run: `GGML_NATIVE=OFF cargo test --manifest-path Cargo.toml -p openhuman emergency_guard_blocks_when_engaged`
Expected: PASS; `middleware.rs` compiles with the new guard.

- [ ] **Step 4: Commit.**

```bash
git add src/openhuman/tinyagents/middleware.rs
git commit -m "feat(emergency): approval middleware refuses external-effect tools while halted (#4255)"
```

---

## Task 8: Enforcement chokepoint 2 — accessibility input blocked while halted

**Files:**
- Modify: `src/openhuman/screen_intelligence/ops.rs` (`accessibility_input_action`, ~line 152)

**Interfaces:**
- Consumes: `is_engaged_global`; `InputActionResult { accepted, blocked, reason }`; `InputActionParams { action, .. }`.

- [ ] **Step 1: Add the halt check.** Replace the body of `accessibility_input_action` with a guard that blocks while halted, except the `panic_stop` action (a stop must never be blocked by a stop):

```rust
pub async fn accessibility_input_action(
    payload: InputActionParams,
) -> Result<RpcOutcome<InputActionResult>, String> {
    // Emergency stop: refuse desktop input while halted. `panic_stop` is
    // exempt so a stop is never blocked by a stop.
    if payload.action != "panic_stop" && crate::openhuman::emergency_stop::is_engaged_global() {
        tracing::warn!(action = %payload.action, "[emergency] accessibility_input_action blocked — kill switch engaged");
        return Ok(RpcOutcome::single_log(
            InputActionResult { accepted: false, blocked: true, reason: Some("emergency_stop".to_string()) },
            "screen intelligence input blocked by emergency stop",
        ));
    }
    let result = screen_intelligence::global_engine()
        .input_action(payload)
        .await?;
    Ok(RpcOutcome::single_log(
        result,
        "screen intelligence input action processed",
    ))
}
```

- [ ] **Step 2: Write a unit test.** Add to the `#[cfg(test)] mod tests` in `screen_intelligence/ops.rs` (the file already has tests like `accessibility_stop_session_is_tolerant_of_no_reason`):

```rust
    #[tokio::test]
    async fn input_action_blocked_while_emergency_engaged() {
        use crate::openhuman::emergency_stop::EmergencyStop;
        let stop = EmergencyStop::init_global();
        stop.engage(Some("test".into()), "user", 0);
        let params = InputActionParams { action: "click".into(), x: Some(1), y: Some(1), button: None, text: None, key: None, modifiers: None };
        let out = accessibility_input_action(params).await.unwrap();
        assert!(!out.value.accepted);
        assert!(out.value.blocked);
        assert_eq!(out.value.reason.as_deref(), Some("emergency_stop"));
        stop.clear();
    }

    #[tokio::test]
    async fn panic_stop_passes_even_while_emergency_engaged() {
        use crate::openhuman::emergency_stop::EmergencyStop;
        let stop = EmergencyStop::init_global();
        stop.engage(None, "user", 0);
        let params = InputActionParams { action: "panic_stop".into(), x: None, y: None, button: None, text: None, key: None, modifiers: None };
        // Should not be short-circuited by the emergency guard (reaches the engine).
        let _ = accessibility_input_action(params).await;
        stop.clear();
    }
```

(Confirm `out.value` accessor matches `RpcOutcome`'s public field, as in Task 5.)

- [ ] **Step 3: Build + test.**

Run: `GGML_NATIVE=OFF cargo test --manifest-path Cargo.toml -p openhuman input_action_blocked_while_emergency_engaged`
Expected: PASS.

- [ ] **Step 4: Commit.**

```bash
git add src/openhuman/screen_intelligence/ops.rs
git commit -m "feat(emergency): accessibility_input_action refuses input while halted (#4255)"
```

---

## Task 9: JSON-RPC E2E — stop → status → resume

**Files:**
- Modify: `tests/json_rpc_e2e.rs` (add a test mirroring existing approval E2E tests)

**Interfaces:**
- Consumes: the JSON-RPC dispatcher via the existing E2E harness in `tests/json_rpc_e2e.rs`.

- [ ] **Step 1: Read an existing E2E test** in `tests/json_rpc_e2e.rs` (e.g. an approval one) to copy the harness setup (how it boots the core, obtains a client, and calls `openhuman.<method>`).

- [ ] **Step 2: Write the E2E test** following that harness's exact helper signatures:

```rust
// Emergency stop: status(not halted) → stop → status(halted) → resume → status(not halted).
#[tokio::test]
async fn emergency_stop_roundtrip_over_rpc() {
    let harness = /* boot core per existing pattern in this file */;
    let s0 = harness.call("openhuman.emergency_status", serde_json::json!({})).await;
    assert_eq!(s0["engaged"], serde_json::json!(false));

    let stopped = harness.call("openhuman.emergency_stop", serde_json::json!({ "reason": "e2e" })).await;
    assert_eq!(stopped["engaged"], serde_json::json!(true));

    let s1 = harness.call("openhuman.emergency_status", serde_json::json!({})).await;
    assert_eq!(s1["engaged"], serde_json::json!(true));

    let resumed = harness.call("openhuman.emergency_resume", serde_json::json!({})).await;
    assert_eq!(resumed["engaged"], serde_json::json!(false));
}
```

Adapt `harness`/`call` to the file's real helpers (method name mapping `emergency.stop` → `openhuman.emergency_stop` is handled by the dispatcher; verify against how approval methods are invoked in this file).

- [ ] **Step 3: Run.**

Run: `bash scripts/test-rust-with-mock.sh --test json_rpc_e2e emergency_stop_roundtrip_over_rpc`
Expected: PASS.

- [ ] **Step 4: Commit.**

```bash
git add tests/json_rpc_e2e.rs
git commit -m "test(emergency): json-rpc e2e for stop/status/resume (#4255)"
```

---

## Task 10: Web socket bridge — `AutomationHalted`/`Resumed` → `automation_halt` event

**Files:**
- Modify: `src/openhuman/channels/providers/web/event_bus.rs` (add `AutomationHaltSubscriber`, `register_automation_halt_subscriber`)
- Modify: `src/openhuman/channels/runtime/startup.rs` (call the register fn where `register_approval_surface_subscriber` is called)

**Interfaces:**
- Consumes: `DomainEvent::AutomationHalted/Resumed`; `WebChannelEvent` (read `src/core/socketio` for its fields — the approval bridge sets `event`, `client_id`, `thread_id`, `args`).

- [ ] **Step 1: Broadcast mechanism — CONFIRMED.** Emergency halt is global (not thread-scoped). `emit_web_channel_event` (src/core/socketio.rs:1409) delivers each event to the Socket.IO room named `event.client_id`, and **every** connected client auto-joins the `"system"` room (socketio.rs:438). So to broadcast to all clients, set `client_id: "system".to_string()` and leave `thread_id` empty — the emit code special-cases `client_id == "system"` for single-room delivery. **Do NOT use `..Default::default()` for `client_id`** (empty string → room `""` → reaches nobody). The frontend (Task 14) listens for the `automation_halt` event name globally.

- [ ] **Step 2: Add the subscriber** in `event_bus.rs` (mirror `ApprovalSurfaceSubscriber`), emitting to the `"system"` broadcast room:

```rust
static AUTOMATION_HALT_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

pub fn register_automation_halt_subscriber() {
    if AUTOMATION_HALT_HANDLE.get().is_some() {
        return;
    }
    match crate::core::event_bus::subscribe_global(Arc::new(AutomationHaltSubscriber)) {
        Some(handle) => { let _ = AUTOMATION_HALT_HANDLE.set(handle);
            log::info!("[web-channel] automation-halt subscriber registered — bridges AutomationHalted/Resumed → automation_halt socket event"); }
        None => log::warn!("[web-channel] failed to register automation-halt subscriber — bus not initialized"),
    }
}

struct AutomationHaltSubscriber;

#[async_trait]
impl EventHandler for AutomationHaltSubscriber {
    fn name(&self) -> &str { "channels::web::automation_halt" }
    fn domains(&self) -> Option<&[&str]> { Some(&["system"]) }
    async fn handle(&self, event: &DomainEvent) {
        match event {
            DomainEvent::AutomationHalted { reason, source } => {
                publish_web_channel_event(WebChannelEvent {
                    event: "automation_halt".to_string(),
                    client_id: "system".to_string(), // broadcast room — all clients auto-join it
                    args: Some(serde_json::json!({ "engaged": true, "reason": reason, "source": source })),
                    ..Default::default()
                });
            }
            DomainEvent::AutomationResumed { source } => {
                publish_web_channel_event(WebChannelEvent {
                    event: "automation_halt".to_string(),
                    client_id: "system".to_string(), // broadcast room — all clients auto-join it
                    args: Some(serde_json::json!({ "engaged": false, "source": source })),
                    ..Default::default()
                });
            }
            _ => {}
        }
    }
}
```

- [ ] **Step 3: Register at startup.** In `src/openhuman/channels/runtime/startup.rs`, next to `register_approval_surface_subscriber()`, add `register_automation_halt_subscriber();`.

- [ ] **Step 4: Add a unit test** mirroring `fresh_approval_surface_subscription_returns_some_when_bus_is_ready` if a `fresh_*` helper is warranted; otherwise a minimal test asserting `register_automation_halt_subscriber()` is idempotent (second call is a no-op) after `init_global`.

- [ ] **Step 5: Build + test.**

Run: `GGML_NATIVE=OFF cargo test --manifest-path Cargo.toml -p openhuman automation_halt`
Expected: PASS.

- [ ] **Step 6: Commit.**

```bash
git add src/openhuman/channels/providers/web/event_bus.rs src/openhuman/channels/runtime/startup.rs
git commit -m "feat(emergency): bridge halt/resume events to automation_halt socket event (#4255)"
```

---

## Task 11: Frontend Redux `safetySlice`

**Files:**
- Create: `app/src/store/safetySlice.ts`
- Create: `app/src/store/safetySlice.test.ts`
- Modify: root store (`app/src/store/index.ts` or wherever `configureStore`/`combineReducers` lives) — mount `safety` reducer.

**Interfaces:**
- Produces: `safetyReducer`, actions `setHalt({reason?, since?, source?})`, `clearHalt()`, `hydrateHalt(HaltState)`; selector `selectHalted(state)`, `selectHaltReason(state)`.

- [ ] **Step 1: Write the failing test.** Create `app/src/store/safetySlice.test.ts`:

```ts
import { describe, it, expect } from 'vitest';
import reducer, { setHalt, clearHalt, hydrateHalt } from './safetySlice';

describe('safetySlice', () => {
  it('starts not halted', () => {
    expect(reducer(undefined, { type: '@@init' })).toEqual({ halted: false });
  });
  it('setHalt marks halted with reason/source/since', () => {
    const s = reducer(undefined, setHalt({ reason: 'user', source: 'user', since: 42 }));
    expect(s).toEqual({ halted: true, reason: 'user', source: 'user', since: 42 });
  });
  it('clearHalt resets', () => {
    const halted = reducer(undefined, setHalt({ reason: 'x' }));
    expect(reducer(halted, clearHalt())).toEqual({ halted: false });
  });
  it('hydrateHalt maps a HaltState snapshot', () => {
    const s = reducer(undefined, hydrateHalt({ engaged: true, reason: 'boot', engaged_at_ms: 7, source: 'system' }));
    expect(s.halted).toBe(true);
    expect(s.reason).toBe('boot');
    expect(s.since).toBe(7);
  });
});
```

- [ ] **Step 2: Run — verify fail.** `pnpm test app/src/store/safetySlice.test.ts` → FAIL (module not found).

- [ ] **Step 3: Implement `safetySlice.ts`:**

```ts
import { createSlice, PayloadAction } from '@reduxjs/toolkit';

export interface HaltState {
  engaged: boolean;
  reason?: string;
  engaged_at_ms?: number;
  source?: string;
}

export interface SafetyState {
  halted: boolean;
  reason?: string;
  since?: number;
  source?: string;
}

const initialState: SafetyState = { halted: false };

const safetySlice = createSlice({
  name: 'safety',
  initialState,
  reducers: {
    setHalt(_state, action: PayloadAction<{ reason?: string; source?: string; since?: number }>) {
      return { halted: true, reason: action.payload.reason, source: action.payload.source, since: action.payload.since };
    },
    clearHalt() {
      return { halted: false };
    },
    hydrateHalt(_state, action: PayloadAction<HaltState>) {
      const h = action.payload;
      return h.engaged
        ? { halted: true, reason: h.reason, source: h.source, since: h.engaged_at_ms }
        : { halted: false };
    },
  },
});

export const { setHalt, clearHalt, hydrateHalt } = safetySlice.actions;
export const selectHalted = (state: { safety: SafetyState }) => state.safety.halted;
export const selectHaltReason = (state: { safety: SafetyState }) => state.safety.reason;
export default safetySlice.reducer;
```

- [ ] **Step 4: Mount reducer** in the root store under key `safety` (follow the existing slice-registration pattern — the store already registers `chatRuntime`, `thread`, etc.).

- [ ] **Step 5: Run tests.** `pnpm test app/src/store/safetySlice.test.ts` → PASS. Also `pnpm typecheck`.

- [ ] **Step 6: Commit.**

```bash
git add app/src/store/safetySlice.ts app/src/store/safetySlice.test.ts app/src/store/index.ts
git commit -m "feat(emergency): safetySlice tracks automation-halt state (#4255)"
```

---

## Task 12: Frontend `emergencyApi` client

**Files:**
- Create: `app/src/services/api/emergencyApi.ts`
- Create: `app/src/services/api/emergencyApi.test.ts`

**Interfaces:**
- Consumes: `callCoreRpc` from `coreRpcClient`. **CONFIRMED signature (do not use positional args):** `callCoreRpc<T>({ method, params }): Promise<T>` — it takes a single **object** `{ method: string, params?: object }`. Mirror `app/src/services/api/approvalApi.ts`.
- **CONFIRMED wire-shape:** RPCs that emit a diagnostic log return the CLI envelope `{ result, logs }`; log-less RPCs return a bare value. `emergency_stop`/`emergency_resume` use `RpcOutcome::single_log` (enveloped); `emergency_status` uses `RpcOutcome::new(_, vec![])` (bare). So the client MUST normalize both shapes with an `unwrapValue` helper — copy the one in `approvalApi.ts` (lines ~109-114) verbatim.
- Produces: `emergencyStop(reason?: string): Promise<HaltState>`, `emergencyResume(): Promise<HaltState>`, `emergencyStatus(): Promise<HaltState>`.

- [ ] **Step 1: Read `app/src/services/api/approvalApi.ts`** to copy the exact RPC-call idiom: the object-form `callCoreRpc({ method, params })`, the `unwrapValue<T>` helper, and method-name convention `openhuman.<ns>_<fn>`.

- [ ] **Step 2: Write the failing test.** Create `emergencyApi.test.ts`. `callCoreRpc` is a **named export** of `../coreRpcClient` and is called with an object:

```ts
import { describe, it, expect, vi, beforeEach } from 'vitest';

const call = vi.fn();
vi.mock('../coreRpcClient', () => ({ callCoreRpc: (arg: unknown) => call(arg) }));

import { emergencyStop, emergencyResume, emergencyStatus } from './emergencyApi';

beforeEach(() => call.mockReset());

describe('emergencyApi', () => {
  it('emergencyStop calls openhuman.emergency_stop with reason and unwraps envelope', async () => {
    call.mockResolvedValue({ result: { engaged: true, reason: 'user' }, logs: ['x'] });
    const r = await emergencyStop('user');
    expect(call).toHaveBeenCalledWith({ method: 'openhuman.emergency_stop', params: { reason: 'user' } });
    expect(r.engaged).toBe(true);
    expect(r.reason).toBe('user');
  });
  it('emergencyStop with no reason sends empty params', async () => {
    call.mockResolvedValue({ result: { engaged: true }, logs: [] });
    await emergencyStop();
    expect(call).toHaveBeenCalledWith({ method: 'openhuman.emergency_stop', params: {} });
  });
  it('emergencyResume calls openhuman.emergency_resume', async () => {
    call.mockResolvedValue({ result: { engaged: false }, logs: ['x'] });
    const r = await emergencyResume();
    expect(call).toHaveBeenCalledWith({ method: 'openhuman.emergency_resume', params: {} });
    expect(r.engaged).toBe(false);
  });
  it('emergencyStatus reads bare value (no envelope)', async () => {
    call.mockResolvedValue({ engaged: false });
    const r = await emergencyStatus();
    expect(call).toHaveBeenCalledWith({ method: 'openhuman.emergency_status', params: {} });
    expect(r.engaged).toBe(false);
  });
});
```

- [ ] **Step 3: Run — verify fail.** `pnpm test app/src/services/api/emergencyApi.test.ts` → FAIL.

- [ ] **Step 4: Implement `emergencyApi.ts`** mirroring `approvalApi.ts` (object-form call + `unwrapValue`):

```ts
import { callCoreRpc } from '../coreRpcClient';
import type { HaltState } from '../../store/safetySlice';

/** Normalize the CLI envelope `{ result, logs }` and bare-value shapes. */
const unwrapValue = <T>(raw: unknown): T => {
  if (raw && typeof raw === 'object' && 'result' in (raw as Record<string, unknown>)) {
    return (raw as { result: T }).result;
  }
  return raw as T;
};

export async function emergencyStop(reason?: string): Promise<HaltState> {
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.emergency_stop', params: reason ? { reason } : {} });
  return unwrapValue<HaltState>(raw);
}

export async function emergencyResume(): Promise<HaltState> {
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.emergency_resume', params: {} });
  return unwrapValue<HaltState>(raw);
}

export async function emergencyStatus(): Promise<HaltState> {
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.emergency_status', params: {} });
  return unwrapValue<HaltState>(raw);
}
```

- [ ] **Step 5: Run tests.** PASS + `pnpm typecheck`.

- [ ] **Step 6: Commit.**

```bash
git add app/src/services/api/emergencyApi.ts app/src/services/api/emergencyApi.test.ts
git commit -m "feat(emergency): emergencyApi RPC client (#4255)"
```

---

## Task 13: i18n keys

**Files:**
- Modify: `app/src/lib/i18n/en.ts` and every other locale file at `app/src/lib/i18n/<locale>.ts` (`ar, bn, de, es, fr, hi, id, it, ko, pl, pt, ru, zh-CN`)
- Check: `app/src/lib/i18n/types.ts` — if the translation key type is explicitly enumerated there, add the new keys to it (otherwise `pnpm typecheck` fails). The parity/coverage guard lives at `app/src/lib/i18n/__tests__/coverage.test.ts`.

**Interfaces:**
- Produces: keys `safety.emergencyStop`, `safety.resume`, `safety.haltedTitle`, `safety.haltedBody`, `safety.stopConfirm` (used by Task 14 components).

- [ ] **Step 1: Read `app/src/lib/i18n/en.ts` and `types.ts`** to learn the nesting/key style (flat dotted keys vs nested objects) and whether keys are type-enumerated. Add keys to `en.ts` matching that exact style:

```ts
  safety: {
    emergencyStop: 'Emergency stop',
    resume: 'Resume automation',
    haltedTitle: 'Automation halted',
    haltedBody: 'All desktop automation is stopped. Resume when you are ready.',
    stopConfirm: 'Stop all automation now?',
  },
```

- [ ] **Step 2: Add real translations** to every other locale file (not English placeholders — CI `pnpm i18n:english:check` fails on English left in non-English files). Translate the five strings per locale.

- [ ] **Step 3: Verify parity.**

Run: `pnpm i18n:check && pnpm i18n:english:check`
Expected: PASS (all locales have the keys; no English placeholders).

- [ ] **Step 4: Commit.**

```bash
git add app/src/lib/i18n/locales
git commit -m "i18n(emergency): add safety.* keys across locales (#4255)"
```

---

## Task 14: Emergency Stop button + halted banner + wiring

**Files:**
- Create: `app/src/components/safety/EmergencyStopButton.tsx` (+ `.test.tsx`)
- Create: `app/src/components/safety/AutomationHaltedBanner.tsx` (+ `.test.tsx`)
- Modify: app shell/header to mount both (pick the always-visible chrome — e.g. the Conversations header near the `chat-cancel-generation` control, or `AppShell`).
- Modify: `app/src/services/socketService.ts` — handle `automation_halt` socket event → dispatch `setHalt`/`clearHalt`.
- Modify: boot path (e.g. `CoreStateProvider` or an effect in the shell) — call `emergencyStatus()` once and dispatch `hydrateHalt`.

**Interfaces:**
- Consumes: `emergencyStop`/`emergencyResume`/`emergencyStatus` (Task 12); `setHalt`/`clearHalt`/`hydrateHalt`/`selectHalted`/`selectHaltReason` (Task 11); `useT()`.

- [ ] **Step 1: Write the button test.** `EmergencyStopButton.test.tsx` (mirror an existing component test that wraps a Redux `Provider` + i18n — find one such test to copy providers):

```tsx
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { EmergencyStopButton } from './EmergencyStopButton';
import { renderWithProviders } from '../../test-utils'; // use the repo's existing helper; else inline a store+I18n wrapper

const stop = vi.fn().mockResolvedValue({ engaged: true });
vi.mock('../../services/api/emergencyApi', () => ({ emergencyStop: (...a: unknown[]) => stop(...a) }));

beforeEach(() => stop.mockClear());

describe('EmergencyStopButton', () => {
  it('calls emergencyStop and dispatches halt on click', async () => {
    renderWithProviders(<EmergencyStopButton />);
    fireEvent.click(screen.getByRole('button', { name: /emergency stop/i }));
    await waitFor(() => expect(stop).toHaveBeenCalled());
  });
});
```

- [ ] **Step 2: Verify fail.** `pnpm test EmergencyStopButton` → FAIL.

- [ ] **Step 3: Implement `EmergencyStopButton.tsx`:**

```tsx
import { useCallback } from 'react';
import { useDispatch } from 'react-redux';
import { useT } from '../../lib/i18n/I18nContext';
import { emergencyStop } from '../../services/api/emergencyApi';
import { setHalt } from '../../store/safetySlice';

export function EmergencyStopButton() {
  const t = useT();
  const dispatch = useDispatch();
  const onClick = useCallback(async () => {
    try {
      const state = await emergencyStop('user');
      dispatch(setHalt({ reason: state.reason, source: state.source, since: state.engaged_at_ms }));
    } catch (err) {
      // Fail-visible: still reflect intent locally so the user sees the halt.
      dispatch(setHalt({ reason: 'user', source: 'user' }));
      console.error('[emergency] stop failed', err);
    }
  }, [dispatch]);
  return (
    <button type="button" data-analytics-id="emergency-stop" onClick={onClick} className="/* coral/danger token */">
      {t('safety.emergencyStop')}
    </button>
  );
}
```

- [ ] **Step 4: Implement `AutomationHaltedBanner.tsx`** (renders only when halted; Resume clears):

```tsx
import { useCallback } from 'react';
import { useDispatch, useSelector } from 'react-redux';
import { useT } from '../../lib/i18n/I18nContext';
import { emergencyResume } from '../../services/api/emergencyApi';
import { clearHalt, selectHalted, selectHaltReason } from '../../store/safetySlice';

export function AutomationHaltedBanner() {
  const t = useT();
  const dispatch = useDispatch();
  const halted = useSelector(selectHalted);
  const reason = useSelector(selectHaltReason);
  const onResume = useCallback(async () => {
    try { await emergencyResume(); } finally { dispatch(clearHalt()); }
  }, [dispatch]);
  if (!halted) return null;
  return (
    <div role="alert" data-analytics-id="automation-halted-banner">
      <strong>{t('safety.haltedTitle')}</strong>
      <span>{reason ?? t('safety.haltedBody')}</span>
      <button type="button" data-analytics-id="emergency-resume" onClick={onResume}>
        {t('safety.resume')}
      </button>
    </div>
  );
}
```

- [ ] **Step 5: Write the banner test** (`AutomationHaltedBanner.test.tsx`): renders nothing when not halted; renders + Resume calls `emergencyResume` and clears when halted (preload the store with `setHalt`).

- [ ] **Step 6: Socket handler.** In `socketService.ts`, register a handler for the `automation_halt` event (mirror how `approval_request` is handled): on `{engaged:true}` dispatch `setHalt`, on `{engaged:false}` dispatch `clearHalt`.

- [ ] **Step 7: Boot hydration.** In the shell/boot effect, call `emergencyStatus()` once and dispatch `hydrateHalt(result)` (guard with `isTauri()`/try-catch).

- [ ] **Step 8: Mount** `<EmergencyStopButton />` in the always-visible chrome and `<AutomationHaltedBanner />` near the top of the main content.

- [ ] **Step 9: Run all frontend checks.**

Run: `pnpm test app/src/components/safety && pnpm typecheck && pnpm lint`
Expected: PASS.

- [ ] **Step 10: Commit.**

```bash
git add app/src/components/safety app/src/services/socketService.ts app/src/store app/src/**/*Shell* 2>/dev/null
git commit -m "feat(emergency): stop button + halted banner + socket/boot wiring (#4255)"
```

---

## Task 15: Full verification + coverage gate

- [ ] **Step 1: Rust suite (changed domains).**

Run: `GGML_NATIVE=OFF cargo test --manifest-path Cargo.toml -p openhuman emergency_stop:: && bash scripts/test-rust-with-mock.sh --test json_rpc_e2e emergency_stop_roundtrip_over_rpc`
Expected: all PASS.

- [ ] **Step 2: Rust format + check.**

Run: `cargo fmt --manifest-path Cargo.toml && GGML_NATIVE=OFF cargo check --manifest-path Cargo.toml`
Expected: no diffs, clean check.

- [ ] **Step 3: Frontend suite + quality.**

Run: `pnpm test && pnpm typecheck && pnpm lint && pnpm i18n:check && pnpm i18n:english:check`
Expected: all PASS.

- [ ] **Step 4: Diff coverage sanity.** Ensure the changed Rust lines (ops.rs guards, chokepoints) and changed TS lines (slice, api, components) are exercised by the tests above. Add targeted tests for any uncovered branch (e.g. `emergency_status` when no switch installed → `HaltState::default`). Target ≥80% on changed lines.

- [ ] **Step 5: Update feature docs.** Per AGENTS.md, if this adds a user-facing feature, update `src/openhuman/about_app/` with the Emergency Stop control. Commit.

```bash
git commit -am "docs(about): register emergency stop as a user-facing control (#4255)"
```

- [ ] **Step 6: Manual smoke (optional but recommended).** `pnpm dev:app`, engage Emergency Stop, confirm the banner appears and Resume clears it; confirm an accessibility input while halted returns blocked.

---

## Self-Review (completed by plan author)

- **Spec coverage:** AC "emergency stop cancels pending actions" → Task 5 cascade-deny + a11y stop; "prevents further queued actions until resume" → Tasks 7–8 chokepoints + Task 5 flag; UI control + resume → Tasks 11–14; tests/≥80% coverage → every task is TDD + Task 15. ✔
- **Placeholder scan:** No TBDs. The few "read neighboring file to confirm exact accessor" notes are explicit verification steps (RpcOutcome `.value`, `callCoreRpc` export, `WebChannelEvent` fields, test-provider helper) with the exact file to read — not deferred work. ✔
- **Type consistency:** `HaltState` fields (`engaged`, `reason`, `engaged_at_ms`, `source`) identical across Rust (Task 2) and TS (Task 11/12); RPC method names `openhuman.emergency_{stop,resume,status}` consistent Tasks 6/9/12; event names `AutomationHalted`/`AutomationResumed` consistent Tasks 1/10; socket event `automation_halt` consistent Tasks 10/14. ✔
- **Ordering:** Task 1 (events) precedes Task 5 (publishes them); Tasks 2–4 (module compiles) precede Task 5–6; chokepoints (7–8) after the switch exists; frontend (11–14) independent of Rust after RPC names are fixed. ✔

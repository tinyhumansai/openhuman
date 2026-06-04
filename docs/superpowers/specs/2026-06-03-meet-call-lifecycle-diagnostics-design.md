# Meet Call Lifecycle Diagnostics — Design

**Date:** 2026-06-03
**Issue:** [#2945 — Google Meet agent join flow is unreliable](https://github.com/tinyhumansai/openhuman/issues/2945)
**Slice:** B (Diagnostics-first). Slices A (root-cause) and C (UX flow) are deliberately out of scope and will ship as separate PRs.

## Problem

When a user sends OpenHuman to a Google Meet, the join flow currently has no observable lifecycle past the initial "Joining…" success toast. The reporter describes the agent joining briefly, dropping after ~5 seconds, showing a "rejoining" state, and never recovering.

Investigation found:

- **No rejoin logic exists in OpenHuman.** The "rejoining state" the reporter sees is Meet's own page UI ("Trying to reconnect…") — the wrapper has zero visibility into it.
- **The scanner stops watching after admission.** `meet_scanner::run` finishes at "click captions on" and exits. After that, nothing in our code knows whether the bot is still in the call.
- **No frontend signal past "Joining…".** Only `meet-call:closed` is emitted to the renderer post-launch — no `admitted`, `dropped`, or `failed` events.
- **Logs exist but are scattered.** `log::info!`/`log::warn!` lines use ad-hoc prefixes (`[meet-scanner]`, `[meet-call]`, `[meet-audio]`) so a single grep can't reconstruct one call's story.

This PR addresses issue #2945's acceptance criteria **"Failure diagnostics captured"** and **"Rejoin state resolves → exits with a clear actionable error"**. The other criteria (clear join flow, agent stays joined, rejoin resolves successfully, regression E2E) are slices A and C.

## Approach

**Shell-only beacons + terminal-failure toast.** All lifecycle events originate from the Tauri shell where the lifecycle actually happens (`meet_call`, `meet_scanner`, `meet_audio`); the React frontend subscribes and surfaces terminal failures as toasts. No core changes; no new RPCs; no new `MeetCallRecord` fields.

Alternatives considered and rejected:
- **B — persist outcome on MeetCallRecord:** adds serde-default + store migration + UI work for red rows in Recent Calls. Bigger surface; better as a follow-up once we know which reason codes show up in practice.
- **C — DomainEvent bus mirror:** adds a new `DomainEvent::MeetCallPhase` variant with no current consumer. Premature.

## Event Vocabulary

Two Tauri events, both keyed by `request_id` (existing per-call correlation key):

```text
meet-call:phase   { request_id, phase, detail? }
meet-call:failed  { request_id, phase, reason_code, message }
```

`phase` is a coarse three-state enum — fine-grained sub-phases stay in logs:

| `phase` value | Emitted from | When |
|---|---|---|
| `"joining"` | `meet_call::meet_call_open_window` | Window built, scanner spawn started |
| `"awaiting_admission"` | `meet_scanner::run` | After `"Ask to join"` click succeeds |
| `"joined"` | `meet_scanner::run` | After `wait_for_admission` returns Ok |

`reason_code` (event-emitted set):

| Code | Source |
|---|---|
| `name_input_timeout` | `meet_scanner` Phase 2 (`type_into_named_input("Your name", ...)`) timed out |
| `ask_to_join_timeout` | `meet_scanner` Phase 3 (`wait_and_click_text(["Ask to join", "Join now"])`) timed out |
| `admission_timeout` | `meet_scanner` Phase 4 (`wait_for_admission`) timed out |

`reason_code` (reserved for log/grep symmetry only — surfaced via RPC return or logs, not as an event):

| Code | Source |
|---|---|
| `invalid_url` | `meet_call_open_window` rejects the URL |
| `window_build_failed` | `WebviewWindowBuilder::build()` fails |
| `audio_bind_failed` | `meet_audio::start` returned Err in the spawned task. Logged with `[meet-lifecycle] audio_bind_failed` — **not** emitted as a `meet-call:failed` event because that path races the frontend `subscribeToMeetCallEvents` registration (the spawn fires before `listen()` resolves) and emitting there would poison the per-`request_id` dedup, suppressing the later scanner-side failure that has a guaranteed subscriber. |
| `cancelled` | Reserved for future; user-close already surfaces as `meet-call:closed` |

## Architecture

### Shell side

New file: `app/src-tauri/src/meet_call/lifecycle.rs`

```rust
pub enum Phase { Joining, AwaitingAdmission, Joined }
pub enum ReasonCode {
    InvalidUrl, WindowBuildFailed,
    NameInputTimeout, AskToJoinTimeout, AdmissionTimeout,
    AudioBindFailed, Cancelled,
}

// Both helpers are idempotent per request_id via MeetCallState.terminated set.
pub fn emit_phase<R: Runtime>(app: &AppHandle<R>, request_id: &str, phase: Phase, detail: Option<&str>);
pub fn emit_failed<R: Runtime>(app: &AppHandle<R>, request_id: &str, phase: Phase, reason: ReasonCode, message: &str);

// Pure helper, no AppHandle — fully unit-testable.
pub fn classify_scanner_err(err: &str, phase_hint: Phase) -> ReasonCode;
```

`MeetCallState` (in `meet_call/mod.rs`) grows a third field:

```rust
pub struct MeetCallState {
    inner: Mutex<HashMap<String, String>>,           // existing
    scanner_aborts: Mutex<HashMap<String, AbortHandle>>, // existing
    terminated: Mutex<HashSet<String>>,              // new — dedup for emit_failed
}
```

`emit_failed` consults & inserts into `terminated` before emitting; second call is a no-op + `log::debug!`. Cleared on `WindowEvent::Destroyed` (same handler that already cleans up `inner` and `scanner_aborts`).

### Call sites

| File | Site | Action |
|---|---|---|
| `meet_call/mod.rs` | `meet_call_open_window`, post-`window.build()` | `emit_phase(Joining)` |
| `meet_scanner/mod.rs` | `run`, after `wait_and_click_text("Ask to join")` | `emit_phase(AwaitingAdmission)` |
| `meet_scanner/mod.rs` | `run`, after `wait_for_admission` Ok | `emit_phase(Joined)` |
| `meet_scanner/mod.rs` | `run`, on `type_into_named_input` Err | `emit_failed(AwaitingAdmission, NameInputTimeout, ...)` |
| `meet_scanner/mod.rs` | `run`, on `wait_and_click_text("Ask to join")` Err | `emit_failed(AwaitingAdmission, AskToJoinTimeout, ...)` |
| `meet_scanner/mod.rs` | `run`, on `wait_for_admission` Err | `emit_failed(Joined, AdmissionTimeout, ...)` |
| `meet_audio/mod.rs` | `start`, when called Err is logged | `emit_failed(Joined, AudioBindFailed, ...)` |

Existing `log::info!` / `log::warn!` lifecycle lines get their prefix swapped to `[meet-lifecycle]` and gain stable `request_id=` and `phase=` fields. Non-lifecycle logs (e.g. `dump_aria_labels`, cookie-clear chatter) keep their current prefixes.

### Frontend side

New helper in `app/src/services/meetCallService.ts`:

```ts
type MeetCallPhase = 'joining' | 'awaiting_admission' | 'joined';
type MeetCallReasonCode =
  | 'name_input_timeout' | 'ask_to_join_timeout' | 'admission_timeout' | 'audio_bind_failed';

export function subscribeToMeetCallEvents(
  requestId: string,
  handlers: {
    onPhase?: (phase: MeetCallPhase, detail?: string) => void;
    onFailed?: (phase: MeetCallPhase, reason: MeetCallReasonCode, message: string) => void;
  },
): () => void;
```

`MeetingBotsCard.tsx` subscribes after `joinMeetCall` resolves, scoped to the returned `request_id`. On `onFailed`, fires `onToast({ type: 'error', title: ..., message })`. The subscription is auto-disposed when the existing `meet-call:closed` listener for the same `request_id` fires, or on component unmount.

i18n: add `skills.meetingBots.failed.{name_input_timeout, ask_to_join_timeout, admission_timeout, audio_bind_failed}` keys plus a generic fallback in `app/src/lib/i18n/en.ts`. Mirror in all 13 locale files with real translations per the i18n rule.

## Error Handling & Races

- **Idempotency:** `MeetCallState.terminated` dedup ensures one terminal toast per call even if two terminal sites trip simultaneously (e.g. scanner timeout + audio bind error in the same second).
- **Ordering:** Phase events are monotonic per-call. Listeners receive events in `emit()` order; the frontend only cares about the *latest* phase before `closed`/`failed`.
- **Cancellation:** User-close already aborts the scanner mid-await (existing behavior). No `failed` event fires — the existing `meet-call:closed` is the signal.
- **Subscribe-after-launch:** `joinMeetCall` resolves once the window is built; the frontend subscribes immediately after. The first emitted event (`joining` from `meet_call_open_window`) actually fires *before* `joinMeetCall` returns, so it won't be observed — that's fine, the toast only consumes `failed`. `invalid_url` / `window_build_failed` surface via the rejected RPC promise, not as events; the existing `catch` block in `handleSubmit` already toasts.
- **Multiple concurrent calls:** `meet_call_open_window` already enforces single-call invariant; `terminated` is keyed by `request_id` so even if the invariant loosens, no cross-call leakage.

## Testing

### Rust (cargo / cargo-llvm-cov)

- `lifecycle.rs` — unit tests for `Phase` / `ReasonCode` serde stability, `MeetCallState.terminated` insert-once semantics, and `classify_scanner_err` mapping (one test per `(err_substring, phase_hint) → expected ReasonCode` row).
- `meet_call/mod.rs` — extend existing `#[cfg(test)] mod tests` with a `meet_call_state_terminated_dedups` test mirroring the existing `meet_call_state_scanner_aborts_insert_and_remove` pattern.
- No CDP mock harness for `meet_scanner` — the only new logic in that file is the `Err → emit_failed` mapping, which is unit-tested via `classify_scanner_err`. The scanner phase code itself is unchanged.

### Vitest

- `app/src/services/meetCallService.test.ts` — unit test for `subscribeToMeetCallEvents`: mocks `@tauri-apps/api/event.listen` (returns a fake `UnlistenFn`), asserts both `meet-call:phase` and `meet-call:failed` listeners register, and that the returned disposer calls both unlisten fns.
- `app/src/components/skills/__tests__/MeetingBotsCard.test.tsx` — add tests:
  - Stubs `subscribeToMeetCallEvents`, captures `onFailed` callback, invokes it with `admission_timeout`, asserts `onToast` called with `type: 'error'`.
  - Negative case: `onPhase` invoked with `joined` does not fire an error toast.

### E2E

Real Google Meet failure modes can't be reproduced in CI. The PR body will explicitly call this out as deferred, with the rationale that this PR creates the structured-event surface that makes a future mocked-emit E2E feasible.

### Coverage

The diff is small (~150 lines of new logic across lifecycle.rs + classify_scanner_err + subscribeToMeetCallEvents + MeetingBotsCard subscription). The tests above land ≥80% diff coverage on changed lines for both Rust and TS. Log-prefix swap edits are formatting-only.

## Out of Scope (will be separate PRs)

- **Slice A — root cause of the 5-second drop.** Needs a local repro, then likely a post-admission CDP watcher and bounded-retry state machine. The beacons + log discipline this PR adds are the prerequisite.
- **Slice C — UX clarity of the join flow & display-name prompt.** PR #3034 (merged) already addressed display-name pre-fill from Persona settings. Remaining UX work is independent of the diagnostics here.
- **Persisted outcome on `MeetCallRecord`.** Approach B above. Best follow-up once reason-code distribution is known from real telemetry.
- **Real E2E for early-disconnect.** Requires either a mock-Meet harness or a Meet-page stub; neither exists today.

## Acceptance Criteria (this PR)

- New Tauri events `meet-call:phase` and `meet-call:failed` emit per the table above; each existing scanner phase has a corresponding event.
- `[meet-lifecycle]` log prefix applied to all lifecycle log lines in `meet_call`, `meet_scanner`, `meet_audio`; each carries `request_id=` and `phase=` fields.
- Frontend `MeetingBotsCard` surfaces an error toast on terminal failure via the existing `onToast` channel, with localized reason text in all 13 locale files.
- `pnpm i18n:check` and `pnpm i18n:english:check` pass.
- Rust + Vitest unit tests cover the new logic; diff coverage ≥80%.
- PR body explicitly defers real E2E with rationale.

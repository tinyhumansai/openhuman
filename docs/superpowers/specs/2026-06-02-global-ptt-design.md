# Global Push-to-Talk Hotkey — Design

**Issue:** [tinyhumansai/openhuman#3090](https://github.com/tinyhumansai/openhuman/issues/3090) — "Global push-to-talk keybind + screen share while tabbed out / in background."

**Scope of this spec:** the *push-to-talk* half only. Background screen capture for the agent is acknowledged in the issue and tracked as a follow-up PR — same domain (voice / agent context), different surface area (host-screen sampling, fullscreen-game compatibility, image-token budget). Keeping them separate keeps each PR reviewable and coverage-gateable.

**Outcome:** the user holds a configurable global hotkey while OpenHuman is *not* the focused window (mid-game, in their IDE, on a Slack call), speaks, releases the key, and the agent answers via TTS — without OpenHuman ever stealing focus.

---

## Goals

- A user-configurable hold-to-talk hotkey that works while OpenHuman is in the background.
- Mic opens on press, closes on release; transcript is auto-posted to the active chat thread and the agent's reply is spoken aloud.
- Audible + visual feedback (chime + small always-on-top overlay) so the user knows the mic is hot without alt-tabbing.
- Works on macOS, Windows, and Linux/X11 in v1. Wayland: documented unsupported with a clear in-app message.

## Non-goals (v1)

- Background screen capture for the agent. (Follow-up issue spawned from #3090.)
- Streaming partial transcripts during the hold.
- Per-thread PTT routing (always routes to the active thread).
- A DXGI-exclusive-fullscreen overlay workaround. (Documented caveat only; chime still plays.)
- Toggle-style PTT (we ship hold-to-talk only — the existing dictation toggle remains for press-once-press-again users).

---

## Architecture overview

```
[User holds hotkey]
       │
[Tauri shell: tauri-plugin-global-shortcut]
       │  ShortcutState::Pressed
       ▼
[app/src-tauri/src/ptt_hotkeys.rs]
       │  emit("ptt://start", { session_id })
       ▼
[app/src/services/pttService.ts]                  ─┐
       │  voice/audio_capture.start                │  hold phase
       │  playChime("open")                        │
       │  invoke("show_ptt_overlay", { active })   │
       │  armWatchdog(10s)                         │
                                                  ─┘

[User releases hotkey]
       │  ShortcutState::Released
       ▼
[ptt_hotkeys.rs] emit("ptt://stop", { session_id })
       │
[pttService.onStop]
       │  voice/audio_capture.finalize → Buffer
       │  playChime("close") + hide overlay
       │  dictationListener.transcribe(buf) → text
       │  chatRuntime.sendMessage({ text, speakReply: true, source: "ptt" })
       ▼
[Core: openhuman.channel_web_chat]
       │  normal agent turn
       │  on assistant final text:
       │      voice::reply_speech.synthesize_and_play(text)   // if speak_reply
       ▼
[User hears reply; OpenHuman window state never changes]
```

The bulk of the work is in the **Tauri shell** (hotkey + overlay window) and the **renderer service layer** (state machine + glue). The Rust core gets exactly one additive change: a `speak_reply: bool` flag on `channel.web_chat` so TTS reply routing doesn't require the renderer to be focused or even running its normal chat UI.

---

## Components

### Tauri shell — `app/src-tauri/src/`

#### `ptt_hotkeys.rs` *(new)*

Owns global hotkey registration for PTT. Mirrors `dictation_hotkeys.rs` in shape, with two key differences: it listens for **both** `Pressed` and `Released`, and rejects pure-modifier shortcuts.

```rust
pub(crate) struct PttHotkeyState {
    pub(crate) shortcut: Mutex<Vec<String>>,   // expanded variants registered
    pub(crate) is_held: AtomicBool,            // CAS-guarded press/release
    pub(crate) session_counter: AtomicU64,
}

pub(crate) fn expand_ptt_shortcuts(shortcut: &str) -> Result<Vec<String>, PttError>;
//   - returns Err(EmptyShortcut)         if trimmed empty
//   - returns Err(ModifierOnlyShortcut)  if every token is a modifier (Ctrl/Cmd/Shift/Alt/Meta)
//   - returns Err(InvalidShortcut(...))  if the plugin parser rejects it
//   - otherwise returns 1 or 2 expanded variants (macOS CmdOrCtrl → [Cmd, Ctrl])

pub(crate) enum PttError {
    EmptyShortcut,
    ModifierOnlyShortcut,
    InvalidShortcut(String),
    AccessibilityRequired,            // macOS
    ShortcutInUse(String),            // Windows
    UnsupportedOnWayland,
    ConflictsWithDictation(String),
    RegistrationFailed(String),
}
```

#### `lib.rs` — two new IPC commands

```rust
#[tauri::command]
async fn register_ptt_hotkey(app: AppHandle<AppRuntime>, shortcut: String) -> Result<(), String>;

#[tauri::command]
async fn unregister_ptt_hotkey(app: AppHandle<AppRuntime>) -> Result<(), String>;
```

Behavior on `register_ptt_hotkey`:

1. Expand & validate via `expand_ptt_shortcuts`.
2. Check overlap with the currently-registered dictation shortcut(s); on overlap return `ConflictsWithDictation`.
3. Unregister any previously-registered PTT shortcut (rollback-safe — same pattern as the dictation registration).
4. Register each expanded variant with a closure that:
   - On `Pressed`: CAS `is_held: false → true`; on success, increment `session_counter` and emit `ptt://start { session_id }`. On failure (CAS lost — auto-repeat or stuck state), drop.
   - On `Released`: CAS `is_held: true → false`; on success, emit `ptt://stop { session_id }` with the *current* counter value. On failure, drop.
5. Persist the registered variants in `PttHotkeyState`.

`unregister_ptt_hotkey` unregisters all currently-registered variants and clears state. Also called on shutdown (`unregister_all` already covered by the plugin's drop).

#### `ptt_overlay.rs` *(new)* — dedicated overlay window

Lazy-create-on-first-register, destroyed on `unregister`. Window config:

| Field | Value |
| --- | --- |
| `label` | `"ptt-overlay"` |
| `url` | `/#/ptt-overlay` (HashRouter route, mounted only in this window) |
| `decorations` | `false` |
| `transparent` | `true` |
| `always_on_top` | `true` |
| `skip_taskbar` | `true` |
| `focus` | `false` (never accepts focus) |
| `resizable` | `false` |
| `shadow` | `false` |
| `visible_on_all_workspaces` | `true` |
| `accept_first_mouse` | `false` |
| `size` | `160 × 56` |
| `position` | bottom-right of primary display, 24px inset (hard-coded in v1) |

IPC command: `show_ptt_overlay({ active: bool, session_id: u64 })` — hides/shows the window with a 250ms fade on close. Window-local React state in `/#/ptt-overlay` toggles a pulsing red dot when `active: true`.

### Rust core — `src/openhuman/`

#### `voice/bus.rs` *(new)*

Per the canonical module shape, the voice domain currently has no `bus.rs`. Add one with a single subscriber-less event publisher and a new variant on `DomainEvent`:

```rust
// in src/core/event_bus/events.rs
pub enum VoiceEvent {
    PttTranscriptCommitted {
        thread_id: ThreadId,
        session_id: u64,
        text_len: usize,             // never log raw transcript
        held_ms: u64,
        finalized_by_watchdog: bool,
    },
    // ...future variants
}

// in DomainEvent
Voice(VoiceEvent),
```

Subscribers will be added in the follow-up screen-capture PR (the screen-intelligence domain will hook here to grab a frame when a PTT turn commits). For v1 we publish, nobody subscribes — the test asserts publish reaches a test subscriber.

#### Chat-send schema — `speak_reply` flag

The user→agent ingress RPC is **`openhuman.channel_web_chat`** (web channel provider — `src/openhuman/channels/providers/web.rs`, schema in `schemas("chat")`, handler `channel_web_chat`, dispatch through `start_chat`). The frontend already calls this from `app/src/services/chatService.ts::chatSend`. Three additive optional fields:

```rust
// In the channel.web_chat input schema (web.rs schemas())
#[serde(default)]
pub speak_reply: Option<bool>,
#[serde(default)]
pub source: Option<String>,        // "ptt" | "dictation" | "type" | ...
#[serde(default)]
pub session_id: Option<u64>,       // PTT correlation key
```

Non-breaking — all fields `Option`. The flags flow through `channel_web_chat → start_chat → spawn_progress_bridge`. The progress bridge buffers `AgentProgress::TextDelta` chunks during the turn; on `AgentProgress::TurnCompleted`, if `speak_reply == Some(true)`, it calls `voice::reply_speech::synthesize_and_play(buffered_text).await`. This is the **only** Rust-core code path change beyond the schema and the bus event.

`source` and `session_id` are persisted on the user message metadata (via the message-record path already used by `start_chat`) and included in the `VoiceEvent::PttTranscriptCommitted` bus event for the screen-capture follow-up PR.

#### `about_app` capability catalog

Add entry:

```rust
Capability {
    id: "voice.ptt",
    label: "Global push-to-talk",
    supported_on: &[Platform::MacOS, Platform::Windows, Platform::LinuxX11],
    requires: &["microphone", "global_shortcut"],
}
```

### Frontend — `app/src/`

#### `services/pttService.ts` *(new singleton)*

State machine:

```
Idle ──[ptt://start]──▶ Capturing ──[ptt://stop]──▶ Finalizing ──▶ Idle
  ▲                         │
  │                         ├──[10s no stop]──▶ Finalizing (watchdog=true)
  │                         │
  │                         └──[mic-fail / preempt / register]──▶ Aborted ──▶ Idle
```

API surface:

```ts
interface PttService {
  init(): void;                        // subscribes to Tauri ptt://* events
  destroy(): void;
  // exposed for tests:
  onStart(session_id: number): Promise<void>;
  onStop(session_id: number): Promise<void>;
  cancel(reason: "preempted_by_ptt" | "mic_failure" | "user_cancel"): void;
}
```

`onStart` (in order):
1. If a session is already active → call `cancel("preempted_by_ptt")`.
2. `playChime("open")`.
3. `invoke("show_ptt_overlay", { active: true, session_id })`.
4. `voice/audio_capture.start({ session_tag: "ptt:" + session_id })`.
5. `armWatchdog(10_000, () => this.onStop(session_id))`.

`onStop`:
1. Disarm watchdog.
2. `const buf = await voice/audio_capture.finalize()`.
3. `playChime("close")`.
4. `invoke("show_ptt_overlay", { active: false, session_id })`.
5. If `buf.duration_ms < 250` → drop session, play `"no-speech"` double-click chime, log `dropped_reason: "empty_audio"`, return.
6. `const text = await dictationListener.transcribe(buf)`.
7. If `!text.trim()` → drop, log `dropped_reason: "empty_transcript"`, return.
8. Resolve `activeThreadId`:
   - If `chatRuntime.activeThread` exists → use it.
   - Else → create a new thread titled `"Voice"` via `openhuman.thread_create`, mark `source: "ptt"`, use its ID.
9. `chatRuntime.sendMessage({ threadId, body: text, metadata: { source: "ptt", session_id }, speakReply: state.ptt.speakReplies })`.
10. Zero the audio buffer.

`cancel`:
- Disarm watchdog, finalize-and-discard the audio buffer (zero it), hide overlay, play error chime, log with reason. No chat message posted.

Errors during the session — handled per the table in **§ Error handling** below.

#### `store/slices/ptt.ts` *(new redux slice)*

```ts
interface PttState {
  shortcut: string | null;       // null = unbound (default)
  speakReplies: boolean;         // default true
  showOverlay: boolean;          // default true
  isHeld: boolean;               // not persisted
}
```

Persisted (except `isHeld`) via the existing redux-persist config. Re-registers the hotkey on rehydration via a sibling `useEffect` to the existing dictation init.

#### `pages/settings/voice/PttSettingsPanel.tsx` *(new)*

- Hotkey-capture widget (same component family as the dictation key picker).
- Toggle: "Speak agent replies" (`speakReplies`).
- Toggle: "Show overlay while held" (`showOverlay`).
- Inline help: "Push-to-talk is off — pick a hotkey to enable." when `shortcut == null`.
- Inline error: surfaces `PttError::ConflictsWithDictation`, `ShortcutInUse`, `AccessibilityRequired` (with a "Open Accessibility settings" button on macOS), `UnsupportedOnWayland`.
- Inline hint: "In exclusive-fullscreen games the overlay won't render — you'll only hear the chime. Switch to borderless fullscreen for the overlay."

#### `pages/PttOverlayPage.tsx` *(new — rendered only in the overlay window)*

Borderless 160×56 region: small mic glyph, label ("Listening…"), pulsing red dot when `state.active`. Reads `active` from a local React state updated by a `useEffect` that listens for `show_ptt_overlay`-relayed events. No redux access — the overlay window has its own React root.

#### `ChatRuntimeProvider` — forward `speak_reply`

`chatService.chatSend` (already the single call site for `openhuman.channel_web_chat`) accepts `speakReply?: boolean`, `source?: string`, `sessionId?: number` and forwards them as the new optional fields. `ChatRuntimeProvider`'s `sendMessage` plumbs them through from `pttService`.

#### Chimes

- `app/src/assets/audio/ptt-open.wav` — short rising tone, ~80ms.
- `app/src/assets/audio/ptt-close.wav` — short falling tone, ~80ms.
- `app/src/assets/audio/ptt-error.wav` — double-click, ~120ms.
- `app/src/assets/audio/README.md` — CC0 attribution.

LUFS-normalized to roughly match the existing in-app notification sound. Played via a plain `Audio` element from `pttService`.

#### i18n

New keys under a `pttSettings` / `pttOverlay` namespace in `app/src/lib/i18n/en.ts`, real translations added to all 13 non-English locale files (`ar`, `bn`, `de`, `es`, `fr`, `hi`, `id`, `it`, `ko`, `pl`, `pt`, `ru`, `zh-CN`). `pnpm i18n:check` and `pnpm i18n:english:check` gate this.

---

## Data flow / sequence diagram

See the architecture overview above. The key invariants:

- **No focus stealing.** No window is `show()`-ed with focus; `show_ptt_overlay` shows a `focus: false` window. The agent reply plays via TTS without any window-state mutation.
- **Single mic at a time.** `voice::audio_capture` enforces this. PTT preempts in-flight dictation; dictation cannot start during a PTT session.
- **Session ID is the correlation key.** Logged in shell + renderer + bus event + chat metadata.

---

## Error handling

| Failure | Behavior |
| --- | --- |
| Mic permission denied (`MicPermissionDenied`) | Error chime, hide overlay, log `[ptt] mic_denied`. Next time the user opens `/settings/voice`, a sticky banner links to OS mic settings. No mid-game modal. |
| Mic stream drops mid-session (USB unplug) | `cancel("mic_failure")`. No chat message posted. |
| STT call fails (network / model timeout) | Post message anyway as `[Voice — transcription failed]` so the user has a breadcrumb. Subsequent agent turn handles it normally. |
| Agent turn errors | Existing chat-error UI. TTS reply just doesn't play. Overlay already hidden by this point. |
| `ptt://stop` never arrives (OS swallowed release) | 10s watchdog finalizes. Session tagged `finalized_by_watchdog: true`. Logged at `warn`. |
| App backgrounded during hold | Hotkey still fires (global). Overlay still shows. Chime still plays. By design. |
| Empty / sub-threshold audio (< 250ms) | Drop session, play `no-speech` chime, log `dropped_reason: "empty_audio"`. No message posted. |
| Empty transcript (STT returned blank) | Same as above with `dropped_reason: "empty_transcript"`. |
| Shortcut conflict with dictation | Registration returns `ConflictsWithDictation`. Settings panel shows the inline error. |
| Wayland session | `UnsupportedOnWayland`. Settings panel surfaces a clear message. Logged once per session. |

**Logging** (per the debug-logging rule): all logs use `[ptt]` prefix. Fields per session: `session_id`, `shortcut`, `held_ms`, `transcript_len`, `dropped_reason`, `finalized_by_watchdog`. PII-safe — never log transcript text or audio buffers, only lengths/durations. Audio buffers are zeroed after finalize.

**Telemetry**: one new analytics event `ptt_session` mirroring the log fields (no transcript), gated by the existing analytics opt-in.

---

## Configuration

- **No `Config` TOML schema change.** All PTT settings live in the renderer's `ptt` redux slice (persisted), mirroring how dictation is configured today.
- **Default `shortcut: null`** (unbound). No hard-coded default key — every possible default conflicts with something common.
- **Default `speakReplies: true`**, **`showOverlay: true`**.
- **Boot path:** on rehydration, if `state.ptt.shortcut` is non-null, call `register_ptt_hotkey`. On settings change, unregister-then-register. Independent of the existing dictation init.

---

## Migration

Brand-new state. No migration. Existing users on `0.53.45+` see the new `/settings/voice` PTT section after upgrade with everything default-off until they bind a key.

---

## Testing

| Layer | What | Where |
| --- | --- | --- |
| Rust unit | `expand_ptt_shortcuts`: empty, modifier-only, valid combos, `CmdOrCtrl` expansion (dual-variant on macOS, single on Win/Linux) | `app/src-tauri/src/ptt_hotkeys.rs` inline `#[cfg(test)]` |
| Rust unit | `speak_reply` / `source` / `session_id` round-trip through `channel.web_chat` schema serde; default behavior unchanged when all omitted | `src/openhuman/channels/providers/web_tests.rs` |
| Rust unit | `DomainEvent::Voice::PttTranscriptCommitted` publishes; test subscriber receives it | `src/openhuman/voice/bus.rs` inline tests |
| Rust E2E | `tests/json_rpc_e2e.rs` — call `channel.web_chat` with `speak_reply: true` and assert `reply_speech::synthesize_and_play` is invoked via a test seam at the progress-bridge's `TurnCompleted` boundary | `tests/json_rpc_e2e.rs` extension |
| Vitest unit | `pttService` state machine: start→stop happy path, watchdog timeout, empty-audio drop, empty-transcript drop, dictation-preempt, double-press idempotency, mic-permission-denied path | `app/src/services/pttService.test.ts` (new) |
| Vitest unit | `ptt` redux slice: shortcut set/clear, toggle settings, rehydration | `app/src/store/slices/ptt.test.ts` (new) |
| Vitest unit | `PttSettingsPanel` — render, hotkey capture, conflict-with-dictation error, mic-denied banner, Wayland banner | `app/src/pages/settings/voice/PttSettingsPanel.test.tsx` (new) |
| Vitest unit | `PttOverlayPage` — renders idle vs active states, listens for active event | `app/src/pages/PttOverlayPage.test.tsx` (new) |
| i18n gate | All new keys present in all 13 locales, no untranslated English values | `pnpm i18n:check` + `pnpm i18n:english:check` (existing CI) |
| WDIO E2E | Desktop spec: register a hotkey via settings UI, simulate the hotkey via `tauri-driver` key injection, assert overlay window appears, assert chat thread receives a message. STT mocked via the shared mock backend returning a fixed transcript. | `app/test/e2e/specs/ptt-flow.spec.ts` (new) |
| Manual smoke | Hold-while-game-in-foreground on macOS + Windows; mic permission denied flow; Wayland fallback message | PR body checklist |

**Coverage gate.** Every changed line in the new files + the `channel.web_chat` schema delta ships with ≥ 80% diff coverage per the existing merge gate. Untested escape valves (the real `Audio.play()` call, the real `tauri-driver` key injection) are isolated behind thin wrappers that can be mocked.

---

## Out of scope (named explicitly)

- **Background screen capture for the agent** — separate follow-up PR off the same issue.
- **PTT-while-dictation-mid-flight** polish beyond "preempt with reason."
- **DXGI exclusive-fullscreen overlay rendering** — documented caveat only.
- **Streaming partial transcripts during hold.**
- **Per-thread PTT routing** (v1 always uses active thread; if none, creates a `"Voice"` thread).
- **Native platform overlays** (NSWindow / Win32 layered / X11 override-redirect) — Tauri overlay window covers v1 needs.
- **PTT toggle-mode** — out; dictation toggle covers that pattern already.

---

## Open questions

None at spec time. If implementation surfaces blockers (e.g. `tauri-plugin-global-shortcut` `Released` semantics regress on a specific OS version), revisit with a small spec amendment rather than a silent design drift.

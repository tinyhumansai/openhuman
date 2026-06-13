---
description: Desktop companion domain — Clicky-style interaction loop tying hotkey, voice, screen intelligence, LLM, TTS, and visual pointing into a single product experience.
icon: robot
---

# Desktop Companion (`src/openhuman/desktop_companion/`)

The desktop companion orchestrates a Clicky-style interaction loop: hotkey activation, microphone capture, screen context, LLM reasoning, speech synthesis, and visual pointing. It reuses existing building blocks rather than reimplementing them.

## Building blocks

| Module | What it provides | Path |
|--------|-----------------|------|
| **screen_intelligence** | Permission-gated capture sessions, `capture_now()`, `VisionSummary`, `AppContextInfo` | `src/openhuman/screen_intelligence/` |
| **voice** | Hotkey listener (push/tap), audio capture, cloud STT (Whisper), TTS (`reply_speech`) | `src/openhuman/voice/` |
| **meet_agent** | LLM orchestration pattern (STT -> LLM -> TTS), WAV packing | `src/openhuman/meet_agent/` |
| **overlay** | Floating UI surface, attention events, typewriter bubbles | `src/openhuman/overlay/` |
| **provider_surfaces** | Connected-app event queue (`ingest_event`, `list_queue`) | `src/openhuman/provider_surfaces/` |
| **accessibility** | Foreground app context (`foreground_context()`) | `src/openhuman/accessibility/` |

## Module layout

```text
src/openhuman/desktop_companion/
  mod.rs          — module exports (light)
  types.rs        — CompanionState enum, CompanionConfig, ConversationTurn, session param/result types
  session.rs      — singleton session lifecycle, state machine, TTL, conversation history
  pipeline.rs     — STT -> screen context -> LLM -> TTS -> pointing orchestration
  pointing.rs     — [POINT:x,y:label:screenN] tag parser, multi-monitor coordinate mapping
  handoff.rs      — provider-surface queue matching for connected-app actions
  bus.rs          — broadcast channel for CompanionStateChangedEvent
  schemas.rs      — RPC controllers (companion_start_session, companion_stop_session, etc.)
```

## State machine

```text
Idle -> Listening -> Thinking -> Speaking -> Pointing -> Idle
                                    |           |
                                    v           v
                                 Listening   Listening  (interrupt)

Any state -> Error -> Idle (reset)
```

Valid transitions are enforced by `session::is_valid_transition()`. Key paths:

- **Happy path**: Idle -> Listening -> Thinking -> Speaking -> Pointing -> Idle
- **No pointing**: Thinking -> Speaking -> Idle (no POINT tags in response)
- **Interrupt**: Speaking/Pointing -> Listening (user re-activates hotkey)
- **Cancel**: Thinking -> Idle (user cancels mid-think)
- **Error recovery**: Any -> Error -> Idle

## Interaction pipeline

`pipeline.rs` orchestrates a single turn:

1. **Activation** — state transitions to Listening (will be driven by Tauri shell hotkey bridge in PR 2)
2. **STT** — audio samples transcribed via `voice::cloud_transcribe` (Whisper)
3. **Screen context** — `accessibility::foreground_context()` for app name + window title
4. **LLM** — chat-completions via `BackendOAuthClient` with system prompt, screen context, and rolling conversation history (last 20 turns as context)
5. **Parse response** — extract `[POINT:x,y:label:screenN]` tags via `pointing::parse_and_map()`
6. **Handoff check** — scan response for provider keywords, match against `provider_surfaces` queue
7. **TTS** — synthesize speech via `voice::reply_speech` (ElevenLabs)
8. **Pointing** — emit pointing targets for overlay animation
9. **Return to Idle**

The pipeline supports cancellation via `CancellationToken` — the Tauri shell can cancel at any checkpoint (between STT, LLM, TTS stages).

Text input is also supported via `run_text_turn()` which skips STT.

## Session lifecycle

- **One session at a time** — enforced by a process-global `Mutex<Option<CompanionSessionInner>>`
- **Consent required** — `start_session` rejects `consent=false`
- **TTL enforcement** — sessions auto-expire when `status()` detects elapsed TTL
- **Conversation history** — capped at 50 turns, oldest drained on overflow

## RPC surface

Namespace: `companion`. All methods go through the standard controller registry.

| Method | Description |
|--------|-------------|
| `companion_start_session` | Start a session with explicit consent + optional TTL |
| `companion_stop_session` | End the active session |
| `companion_status` | Current state, session info, remaining TTL |
| `companion_config_get` | Read companion configuration |
| `companion_config_set` | Update companion configuration |

## Event bus

`CompanionStateChangedEvent` is broadcast via a `tokio::sync::broadcast` channel (same pattern as `overlay::bus`). Three `DomainEvent` variants route to the `"companion"` domain:

- `CompanionSessionStarted { session_id }`
- `CompanionStateChanged { session_id, state, previous_state }`
- `CompanionSessionEnded { session_id, reason }`

## Pointing system

LLM responses can embed `[POINT:x,y:label:screenN]` tags. `pointing.rs`:

- Parses tags via regex
- Maps screen-relative coordinates to absolute desktop coordinates using `ScreenGeometry`
- Clamps coordinates to screen bounds
- Falls back to screen 0 when the index is out of range
- Strips tags from display text

## Provider-surface handoff

`handoff.rs` scans the clean LLM response text for provider keywords (slack, discord, telegram, etc.) and matches them against items in the `provider_surfaces` queue. When matches are found, `HandoffEvent`s are included in `TurnResult` for the Tauri shell / overlay to surface.

## Platform scope

- **macOS**: Full support — hotkey, screen capture, pointing, TTS, overlay
- **Windows/Linux**: Partial — hotkey works (rdev), screen context stubbed, no pointing

Platform-specific code is gated with `#[cfg(target_os = "macos")]`.

## Testing

| File | Coverage |
|------|----------|
| `session_tests.rs` | Session CRUD, state machine transitions, TTL, consent, conversation history |
| `pipeline_tests.rs` | Turn orchestration, cancellation, input validation, system prompt |
| `pointing_tests.rs` | Tag parsing, coordinate mapping, multi-monitor, edge cases |
| `handoff.rs` (inline) | Keyword matching, empty queue, provider coverage |
| `schemas.rs` (inline) | Controller count, schema field validation |
| `tests/json_rpc_e2e.rs` | Full RPC round-trip: start -> status -> config -> stop |

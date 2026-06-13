# desktop_companion

Clicky-style desktop companion interaction loop. Ties hotkey activation, microphone capture, screen context, LLM reasoning, speech synthesis, and visual pointing into a single product experience. The module is orchestration-only: it composes existing building blocks (`voice` STT/TTS, `meet_agent` LLM/wav helpers, `accessibility` foreground context, `provider_surfaces` queue, the backend chat-completions API) into a per-turn pipeline driven by a single process-global session state machine. `mod.rs` is export-focused; operational code lives in `session.rs`, `pipeline.rs`, `pointing.rs`, and `handoff.rs`.

## Responsibilities

- Own a single process-global companion **session** (only one active at a time) with TTL enforcement, consent gating, and conversation history.
- Drive the **state machine**: `Idle → Listening → Thinking → Speaking/Pointing → Idle`, with validated transitions and `Any → Error` / `Error → Idle`.
- Run a single **interaction turn** (text or audio): STT → screen context → LLM → POINT-tag parse → TTS → pointing, cancellable mid-turn via `CancellationToken`.
- Parse `[POINT:x,y:label:screenN]` tags from LLM output and map screen-relative coordinates to absolute multi-monitor desktop coordinates.
- Detect **provider-surface handoff** opportunities (LLM mentions Slack/Discord/email/etc.) and match against the `provider_surfaces` respond queue.
- Broadcast companion state changes over an internal `tokio::broadcast` bus and publish session lifecycle `DomainEvent`s on the global event bus.
- Expose session lifecycle + config over JSON-RPC under the `companion` namespace.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/desktop_companion/mod.rs` | Export-only: module decls + re-export of the controller schema/registry pair. |
| `src/openhuman/desktop_companion/types.rs` | Serde types: `CompanionState`, `ConversationTurn`, `CompanionConfig`, start/stop params + results, `CompanionSessionStatus`, `CompanionStateChangedEvent`. |
| `src/openhuman/desktop_companion/session.rs` | Session lifecycle + state machine over a `Mutex<Option<...>>` process-global singleton. TTL auto-expiry, transition validation, conversation history (capped at 50 turns). |
| `src/openhuman/desktop_companion/pipeline.rs` | Single-turn orchestration (`run_text_turn`, `run_audio_turn`) and real adapters for STT, LLM chat-completions, and TTS. Defines `TurnResult`. |
| `src/openhuman/desktop_companion/pointing.rs` | POINT-tag regex parser; maps screen-relative coords to absolute desktop coords. Defines `PointTarget`, `ScreenGeometry`, `PointingParseResult`. |
| `src/openhuman/desktop_companion/handoff.rs` | Provider keyword matching against the `provider_surfaces` queue; emits `HandoffEvent`. Token-aware matching to avoid substring false positives. |
| `src/openhuman/desktop_companion/bus.rs` | Process-global `tokio::broadcast` channel for `CompanionStateChangedEvent` (`subscribe_state_changed` / `publish_state_changed`). |
| `src/openhuman/desktop_companion/schemas.rs` | JSON-RPC controller registry: 5 controllers under the `companion` namespace + their handlers. |
| `src/openhuman/desktop_companion/{session,pipeline,pointing}_tests.rs` | Sibling test suites via `#[path]`. `handoff.rs` and `bus.rs` keep inline tests. |

## Public surface

Re-exported from `mod.rs`:

- `all_desktop_companion_controller_schemas()` / `all_desktop_companion_registered_controllers()` — RPC registry pair.

Used by `pipeline.rs` and the Tauri/RPC layer (public within the crate):

- `session::{start_session, stop_session, session_status, transition_state, push_conversation_turn, conversation_history}`
- `pipeline::{run_text_turn, run_audio_turn, TurnResult}`
- `pointing::{parse_and_map, PointTarget, ScreenGeometry, PointingParseResult}`
- `handoff::{check_handoff, HandoffEvent}`
- `bus::{subscribe_state_changed, publish_state_changed}`
- Types: `CompanionState`, `CompanionConfig`, `CompanionSessionStatus`, `StartCompanionSessionParams`, `StopCompanionSessionParams`, etc.

## RPC / controllers

Namespace `companion` (5 controllers), wired into `src/core/all.rs`:

| Method | Inputs | Notes |
| --- | --- | --- |
| `companion.start_session` | `consent: bool` (required), `ttl_secs: u64?` | Fails if `consent=false` or a non-expired session is already active. |
| `companion.stop_session` | `reason: string?` | Returns `stopped=false` if no session active. |
| `companion.status` | (none) | Auto-expires the session inline if TTL exceeded. |
| `companion.config_get` | (none) | Returns `CompanionConfig::default()` — no persistence yet. |
| `companion.config_set` | `hotkey?`, `activation_mode?`, `ttl_secs?`, `capture_screen?`, `include_app_context?` | **Not implemented** — returns an error; changes are not saved. |

There is no `companion.activate` RPC; running a turn (`run_text_turn` / `run_audio_turn`) is invoked from the Tauri shell / hotkey bridge, which must perform the `Idle → Listening` transition first (documented precondition).

## Agent tools

None. This domain owns no `tools.rs` / agent tools.

## Events

Global event bus (`src/core/event_bus`), domain `"companion"`:

- Publishes `DomainEvent::CompanionSessionStarted { session_id, ttl_secs }` on `start_session`.
- Publishes `DomainEvent::CompanionSessionEnded { session_id, reason, turn_count }` on `stop_session` and on TTL auto-expiry (`reason: "ttl_expired"`).
- `DomainEvent::CompanionStateChanged` is defined in `events.rs` but state changes are currently broadcast on the module-local `bus.rs` channel (`CompanionStateChangedEvent`), not the global bus.

The module-local `bus.rs` broadcast (`subscribe_state_changed`) is intended to be bridged to the overlay via Socket.IO as `companion:state_changed`. No `EventHandler` subscribers are registered by this module.

## Persistence

None durable. The active session is held only in a process-global `Mutex<Option<CompanionSessionInner>>` in `session.rs` (in-memory, lost on restart). `CompanionConfig` has no store — `config_get` returns defaults and `config_set` is a stub. No `store.rs`.

## Dependencies

- `crate::core::all` / `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — RPC controller registration and schema types.
- `crate::core::event_bus` — publish session lifecycle `DomainEvent`s.
- `crate::openhuman::memory::EmptyRequest` — empty-params deserialization for no-input RPCs.
- `crate::openhuman::voice::cloud_transcribe` — STT (`transcribe_cloud`).
- `crate::openhuman::voice::reply_speech` — TTS (`synthesize_reply`, ElevenLabs `eleven_turbo_v2_5`).
- `crate::openhuman::meet_agent::wav` — PCM16 → WAV packing for STT upload.
- `crate::openhuman::config::ops::load_config_with_timeout` — load core config for backend calls.
- `crate::openhuman::accessibility::foreground_context` — macOS foreground app/window context (screen context).
- `crate::openhuman::provider_surfaces::{store, types::RespondQueueItem}` — handoff queue lookup.
- `crate::api::{config::effective_backend_api_url, jwt::get_session_token, BackendOAuthClient}` — authenticated chat-completions call to the backend (`/openai/v1/chat/completions`, model `agentic-v1`).
- External crates: `parking_lot`, `tokio` broadcast, `tokio_util::sync::CancellationToken`, `regex`, `uuid`, `chrono`, `base64`, `once_cell`, `serde`/`serde_json`.

## Used by

- `src/openhuman/mod.rs` — declares `pub mod desktop_companion`.
- `src/core/all.rs` — registers the controllers and extends the schema list.
- The Tauri shell / hotkey bridge is the intended runtime driver (performs the `Idle → Listening` transition and invokes the pipeline), though no in-repo Rust caller of `run_*_turn` was found in the core crate beyond tests.

## Notes / gotchas

- **Single active session, process-global.** `start_session` rejects a second session unless the existing one is past TTL (then auto-expired). The `Mutex` itself serializes all session ops — no separate lock.
- **TTL auto-expiry happens in `session_status`** via inline `guard.take()` (not by calling `stop_session`) to avoid a TOCTOU race; it publishes `CompanionSessionEnded` with `reason: "ttl_expired"`.
- **Turn preconditions:** `run_text_turn` / `run_audio_turn` assume the session is already in `Listening`; the caller owns the `Idle → Listening` transition. On cancellation/empty STT the pipeline restores `Idle`.
- **POINT mapping** clamps coords to screen bounds and falls back to screen 0 when the index is out of range or the screens slice is empty (returns raw coords if empty).
- **Handoff is light-touch:** `provider_surfaces` is behaviorally incomplete — handoff just detects matches and emits `HandoffEvent`s; nothing consumes them yet. Single-word keywords use exact token matching ("slacking" won't match "slack"); multi-word ("google meet") use substring; "email"/"gmail" dedupe to provider `gmail`.
- **Screen context is macOS-only** (`gather_screen_context` returns `None` elsewhere).
- **LLM/TTS/STT all require a backend session token**; LLM responses are instructed to avoid markdown (TTS-spoken) and to embed POINT tags.
- Conversation history is capped at 50 turns (oldest drained); the LLM context window uses the last 20 turns.
- `config_set` is a stub and `config_get` returns defaults — config is not yet persisted.

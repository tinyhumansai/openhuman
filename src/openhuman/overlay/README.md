# overlay

Signals pushed from the core to the desktop **overlay window** — a separate Tauri WebView (`OverlayApp.tsx`, configured in `app/src-tauri/tauri.conf.json`) that renders short, non-focus-stealing messages over the desktop. Because the overlay runs in its own JS runtime it cannot share Redux state with the main window; instead it subscribes to a dedicated Socket.IO connection and reacts to events the core broadcasts. This module owns a single fire-and-forget broadcast bus for **attention** events; the Socket.IO transport bridge (`src/core/socketio.rs`) subscribes and forwards them to the overlay window. It is deliberately light: export-focused, one broadcast channel, no persistence and no RPC.

## Responsibilities

- Define the `OverlayAttentionEvent` payload and its `OverlayAttentionTone` visual hint.
- Provide a process-global broadcast channel so any core caller (subconscious loop, heartbeat, screen intelligence, …) can surface a transient overlay message without threading a sender around.
- Expose `publish_attention` (fire-and-forget producer) and `subscribe_attention_events` (consumed by the Socket.IO bridge).
- (STT/dictation overlay activation is driven separately by `voice::dictation_listener`'s `dictation:toggle` / `dictation:transcription` events — not by this module.)

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/overlay/mod.rs` | Module docstring + re-exports only (`publish_attention`, `subscribe_attention_events`, `OverlayAttentionEvent`, `OverlayAttentionTone`). |
| `src/openhuman/overlay/types.rs` | `OverlayAttentionEvent` (message + optional `id` / `tone` / `ttl_ms` / `source`) with builder helpers, and the `OverlayAttentionTone` enum (`neutral` / `accent` / `success`). |
| `src/openhuman/overlay/bus.rs` | `Lazy` `tokio::sync::broadcast` channel (capacity 64) plus `publish_attention` / `subscribe_attention_events`; includes inline `#[cfg(test)]` tests. |

## Public surface

- `OverlayAttentionEvent` — `{ id: Option<String>, message: String, tone: OverlayAttentionTone, ttl_ms: Option<u32>, source: Option<String> }`. Builders: `new(message)`, `with_source(..)`, `with_tone(..)`, `with_ttl_ms(..)`. Only `message` is required.
- `OverlayAttentionTone` — `Neutral` (default), `Accent`, `Success`; serialized lowercase. Frontend maps to bubble colours.
- `publish_attention(event) -> usize` — broadcasts; returns the number of active subscribers that received it (`0` if none, event then dropped). Logs under `[overlay]`.
- `subscribe_attention_events() -> broadcast::Receiver<OverlayAttentionEvent>` — receiver for the bus.

## Events

Not a `DomainEvent` / event-bus (`src/core/event_bus/`) participant. It runs its own standalone `tokio::sync::broadcast` channel. The Socket.IO bridge in `src/core/socketio.rs` (`spawn_web_channel_bridge`, task #3) subscribes via `subscribe_attention_events()` and emits each event to the overlay socket as both `overlay:attention` and `overlay_attention`; it logs and continues on `Lagged` and breaks on `Closed`.

## Persistence

None. State is purely an in-memory broadcast channel; events not consumed when published are dropped.

## Dependencies

- External crates only: `serde` (types), `once_cell::sync::Lazy` + `tokio::sync::broadcast` (bus). No `crate::openhuman::*` or `crate::core::*` imports in the module's own source.

## Used by

- `src/core/socketio.rs` — subscribes to the bus and forwards events to the overlay WebView over Socket.IO.
- `src/openhuman/notifications/bus.rs` — references this module's bus only as a documented pattern to mirror (no code dependency).
- Per the module docstring, intended publishers include the subconscious loop, heartbeat, and screen-intelligence domains via `publish_attention`.

## Notes / gotchas

- **Fire-and-forget:** if the Socket.IO bridge hasn't started or the overlay socket is disconnected, `publish_attention` drops the event and returns `0`. There is no buffering/replay for late subscribers.
- **Channel capacity is 64** — a slow/absent overlay consumer causes `Lagged` drops (logged as a warning by the bridge), not back-pressure.
- Keep `message` short: the overlay types it out character-by-character and auto-dismisses after `ttl_ms` (frontend default when `None`).
- The bridge emits two event names (`overlay:attention` and `overlay_attention`) for client compatibility.

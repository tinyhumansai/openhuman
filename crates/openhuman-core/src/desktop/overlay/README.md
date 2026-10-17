# overlay

Signals pushed from the core to the desktop **overlay surfaces**, separate WebViews ([`app/src/overlay/OverlayApp.tsx`](../../../../../app/src/overlay/OverlayApp.tsx) and the macOS notch pill [`app/src/notch/NotchApp.tsx`](../../../../../app/src/notch/NotchApp.tsx), hosted by [`crates/openhuman-app/src/notch_window.rs`](../../../../openhuman-app/src/notch_window.rs)) that render short, non-focus-stealing messages over the desktop. The standalone `overlay` window is currently disabled in [`crates/openhuman-app/tauri.conf.json`](../../../../openhuman-app/tauri.conf.json); the notch is the live consumer. Because these surfaces run in their own JS runtime they cannot share Redux state with the main window; each subscribes to its own Socket.IO connection and reacts to events the core broadcasts. This module owns a single fire-and-forget broadcast bus for **attention** events; the Socket.IO transport bridge ([`crates/openhuman-rpc/src/server/socketio.rs`](../../../../openhuman-rpc/src/server/socketio.rs)) subscribes and forwards them. It is deliberately light: export-focused, one broadcast channel, no persistence and no RPC.

## Responsibilities

- Define the `OverlayAttentionEvent` payload and its `OverlayAttentionTone` visual hint.
- Provide a process-global broadcast channel so any core caller can surface a transient overlay message without threading a sender around.
- Expose `publish_attention` (fire-and-forget producer) and `subscribe_attention_events` (consumed by the Socket.IO bridge).
- (STT/dictation overlay activation is driven separately by `voice::dictation_listener`'s `dictation:toggle` / `dictation:transcription` events, not by this module.)

## Key files

| File | Role |
| --- | --- |
| [`crates/openhuman-core/src/desktop/overlay/mod.rs`](./mod.rs) | Module docstring + re-exports only (`publish_attention`, `subscribe_attention_events`, `OverlayAttentionEvent`, `OverlayAttentionTone`). |
| [`crates/openhuman-core/src/desktop/overlay/types.rs`](./types.rs) | `OverlayAttentionEvent` (message + optional `id` / `tone` / `ttl_ms` / `source`) with builder helpers, and the `OverlayAttentionTone` enum (`neutral` / `accent` / `success`). |
| [`crates/openhuman-core/src/desktop/overlay/bus.rs`](./bus.rs) | `Lazy` `tokio::sync::broadcast` channel (capacity 64) plus `publish_attention` / `subscribe_attention_events`; tests live in the sibling [`bus_tests.rs`](./bus_tests.rs). |

## Public surface

- `OverlayAttentionEvent`: `{ id: Option<String>, message: String, tone: OverlayAttentionTone, ttl_ms: Option<u32>, source: Option<String> }`. Builders: `new(message)`, `with_source(..)`, `with_tone(..)`, `with_ttl_ms(..)`. Only `message` is required.
- `OverlayAttentionTone`: `Neutral` (default), `Accent`, `Success`; serialized lowercase. Frontend maps to bubble colours.
- `publish_attention(event) -> usize`: broadcasts; returns the number of active subscribers that received it (`0` if none, event then dropped). Logs under `[overlay]`.
- `subscribe_attention_events() -> broadcast::Receiver<OverlayAttentionEvent>`: receiver for the bus.

## Events

Not a `DomainEvent` / bus ([`crates/openhuman-core/src/core/bus.rs`](../../core/bus.rs), [`crates/openhuman-core/src/core/events.rs`](../../core/events.rs)) participant. It runs its own standalone `tokio::sync::broadcast` channel. The Socket.IO bridge in `crates/openhuman-rpc/src/server/socketio.rs` (`spawn_web_channel_bridge`, task #3) subscribes via `subscribe_attention_events()` and emits each event to the overlay socket as both `overlay:attention` and `overlay_attention`; it logs and continues on `Lagged` and breaks on `Closed`.

## Persistence

None. State is purely an in-memory broadcast channel; events not consumed when published are dropped.

## Dependencies

- External crates only: `serde` (types), `once_cell::sync::Lazy` + `tokio::sync::broadcast` (bus). No `crate::*` or `crate::core::*` imports in the module's own source.

## Used by

- `crates/openhuman-rpc/src/server/socketio.rs`: subscribes to the bus and forwards events to the overlay WebView over Socket.IO.
- [`crates/openhuman-core/src/voice/always_on/processor.rs`](../../voice/always_on/processor.rs): `notch_status` publishes "Listening" / "Processing" status messages (`source: "voice"`, with a TTL) that the notch maps to icons. The only in-tree publisher today.
- [`crates/openhuman-core/src/desktop/notifications/bus.rs`](../notifications/bus.rs): references this module's bus only as a documented pattern to mirror (no code dependency).

## Notes / gotchas

- **Fire-and-forget:** if the Socket.IO bridge hasn't started or the overlay socket is disconnected, `publish_attention` drops the event and returns `0`. There is no buffering/replay for late subscribers.
- **Channel capacity is 64**: a slow/absent overlay consumer causes `Lagged` drops (logged as a warning by the bridge), not back-pressure.
- Keep `message` short: the overlay types it out character-by-character and auto-dismisses after `ttl_ms` (frontend default when `None`). The notch treats the literal messages `Listening` and `Processing` as reserved status words.
- The bridge emits two event names (`overlay:attention` and `overlay_attention`) for client compatibility.

## Further reading

- [Parent module README](../README.md)
- [Tauri shell](../../../../../docs/gitbooks/en/developing/architecture/tauri-shell.md)
- [The mascot](../../../../../docs/gitbooks/en/features/mascot/README.md)

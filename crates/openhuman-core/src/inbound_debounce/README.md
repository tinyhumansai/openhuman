# Inbound debounce

`KeyedDebouncer<K, T>` groups bursts of inbound messages per conversation key
into batches and keeps at most one batch per key running. It is a synchronous
state machine with no timer and no task of its own: the caller passes the
current `tokio::time::Instant` into every call, sleeps until `next_wakeup()`,
and reports a finished batch with `complete(key)`. Channel dispatch drives it
from `channels/runtime/dispatch/processor.rs`; other hosts reach it through
`openhuman_embed::inbound`.

## Lifecycle of a key

| Call | Effect |
| --- | --- |
| `push(key, item, settings, now)` | Adds the item to the key's open batch, or opens one. Returns `Opened` for the first item of a quiet window, `Buffered` when it joined or queued, `Ready` when a batch can run now. |
| `poll_ready(now)` | Seals every open batch whose quiet window has passed and returns, oldest first, one batch per idle key. That key is marked running. |
| `complete(key)` | Marks the key idle. A batch collected while it ran waits for the rest of its quiet window, or comes out of the next `poll_ready` if the window has passed. A key with nothing left is forgotten. |
| `next_wakeup()` | The earliest instant a `poll_ready` can yield something. Running keys arm no wakeup. |
| `flush_all(now)` | Closes every quiet window now (shutdown). Running keys keep their batches queued. |
| `drain_all()` | Removes and returns everything, including batches queued behind running keys. |

## Timing

- Every push sets the quiet deadline to `now + window`, where the window is
  `media_debounce` once the batch holds an item with `carries_media()`, and
  `debounce` otherwise.
- A batch never waits past `first push + max_wait`. `max_wait` is raised to
  at least the longer of the two windows.
- Windows and waits longer than `MAX_WINDOW` (one year) are clamped, so an
  oversized setting can never overflow the deadline arithmetic.
- A zero window seals the item at once, so `DebounceSettings::immediate()`
  gives one batch per item while still serializing each key.

## Batching rules

- A push that would take the batch past `max_batch_messages` or
  `max_batch_chars` seals the current batch first and opens a new one.
- An item whose `mergeable()` is false seals the pending batch and becomes a
  batch of its own behind it, so order is kept.
- At most `MAX_TRACKED_KEYS` (2048) keys may have an open quiet window. A new
  key past that limit is sealed at once: it skips the window but is still
  serialized.

## Channel dispatch

The channel loop keys by `history_key`, which is per sender even in group
chats, so different senders are never merged. Runtime commands and approval
replies are handled during admission and never reach the debouncer.
`merge_channel_batch` orders a batch by `(timestamp, receive order)`, uses the
newest message as the base (id, reply target, thread), keeps the last sender
name, and joins the non-blank bodies with newlines, leaving `[IMAGE:…]` and
`[FILE:…]` markers in place. Configuration lives in `[inbound_debounce]`
(`config/schema/inbound_debounce.rs`).

## Credit

The keyed-buffer design (quiet timer reset per message, a hard `max_wait`
deadline, per-key ordering, a tracked-key limit, sealed batches at a message
or character cap, and flushing the pending buffer before an item that cannot
merge) follows the inbound debounce in
[OpenClaw](https://github.com/openclaw/openclaw), used under the MIT License:

```text
MIT License

Copyright (c) 2026 OpenClaw Foundation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

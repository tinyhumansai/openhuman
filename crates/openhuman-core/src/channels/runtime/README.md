# Channel runtime

OpenHuman owns channel configuration, account lifetime, host policy, and the
agent-turn pipeline. The TinyChannels module owns provider construction,
listener supervision, relay transport, and channel-side interaction behavior.
This directory adapts those two sides through the typed TinyBus contract.

## Startup and ownership

[`startup/start_channels.rs`](./startup/start_channels.rs) captures the
session cancellation token, prepares the module runtime limits, hydrates
credentials from the host credential store, and opens a private
`ChannelRuntimeLease`. The module describes configured providers and starts
listeners and the optional relay using the captured configuration. The host
serves typed callbacks for operations it owns, including voice transcription,
allowlist persistence, event publication, and remote-control access. A
malformed or unavailable module response fails closed; there is no local
provider-construction fallback.

Startup registers host event subscribers before accepting inbound work. The
dispatch queue is bounded, and its concurrency limit comes from the module's
`PrepareChannelControl::RuntimeLimits` response. Session cancellation closes
the lease, which asks the module to stop its resources before the host drops
the captured callbacks.

## Inbound path

Native listener and gateway-relayed messages enter the same host dispatch
pipeline. The host preserves authenticated relay provenance separately from
serialized message data. It prepares inbound identity and policy facts through
the module, then applies OpenHuman's routing, approval, and agent-turn policy.
Replies and interactions are sent through the module's typed channel objects;
the host does not reconstruct provider-specific outbound behavior.

The dispatch loop admits each message first: preparation, the
`ChannelMessageReceived` event, runtime commands and approval replies run
immediately. Admissions run concurrently, but their results enter the
debouncer in receive order, so a slow admission never lets a later message
overtake it. What remains goes through the per-conversation debouncer
([`inbound_debounce`](../../inbound_debounce/README.md)) keyed by
`history_key`. Messages from one sender within the quiet window
(`[inbound_debounce]`, 1.5 s by default, 4 s once the batch holds media,
capped at 10 s) become one turn, and a typing indicator starts when the window
opens. A conversation never runs two turns at once: messages that arrive while
its turn runs are collected into one follow-up turn that sees the first turn's
history. Different conversations still run in parallel up to the module's
in-flight limit. The loop holds at most one full batch per turn worker
(`max_batch_messages` × in-flight limit) of admitted messages waiting for a
turn; past that it stops receiving, so the bounded inbound queue pushes back
on senders instead of memory growing. When the inbound queue closes, every buffered message is
flushed before the loop returns.

The always-on `ChannelInboundSubscriber` handles hosted-channel messages
published by the backend socket. It prepares the caller's identity, opens a
private module runtime, and executes progressive delivery against the captured
backend callback. Drafts, typing refreshes, idempotency, and final delivery are
owned by the module. The host callback supplies only the managed REST transport
and credential captured for that turn.

## Account lifetime

Model authentication and channel account lifetime are separate. A local model
can run without a backend session, while logout cancels the channel runtime
captured for the old account. The session owner explicitly closes the runtime
even when shared channel handles remain alive. Provider listeners, dispatch
workers, typing refreshes, and relay tasks stop with their owning module
resources.

## Files

| Path | Purpose |
| --- | --- |
| [`startup.rs`](./startup.rs) and `startup/` | Captures configuration and callbacks, opens the module lease, prepares runtime limits, registers subscribers, and starts the dispatch loop. |
| [`dispatch/`](./dispatch/) | Applies OpenHuman's inbound routing, approval, agent, and reply policy. |
| [`session.rs`](./session.rs) | Tracks the process channel-session cancellation lifetime. |
| [`test_support.rs`](./test_support.rs) | Test/debug dispatch harness; it does not provide a production provider fallback. |

## Tests

Startup and dispatch tests live beside their modules. `module_fixture_tests.rs`
exercises the typed bus callbacks against an in-process TinyChannels module
fixture. The separately maintained released-artifact fixture verifies the
same public bus contract through the pinned module loader.

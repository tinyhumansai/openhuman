# Relay

The hosted-channel relay. A gateway that owns a chat platform's webhook and account link relays each message into the core as the user it belongs to, and delivers the replies back to the platform. The core never talks to the platform itself.

In SaaS the cloud gateway owns the hosted Telegram, iMessage and Discord webhooks and the account links (`openhuman_tinyhumans::hosted::channel_link`). The relay also works in a single-user core, for an embedder or a local relay: there the caller's config is the process's.

## Contract

`openhuman.channel_relay_inbound` (namespace `channel`, beside `channel_web_chat`), on the SaaS user surface (`profiles::surface::USER_METHODS`):

| Param | Rule |
| --- | --- |
| `channel` | Platform name, 1–32 chars of `[a-z0-9_-]`, starting with a letter. `web`, `cli` and `relay` are reserved. Any name in the charset is accepted, not only a fixed list of hosted platforms, so a gateway can add a platform without a core release; the name picks the platform's capabilities (`tinychannels_bus::capabilities_for`). |
| `chat_id`, `sender_id`, `message_id` | 1–128 chars of printable ASCII, without spaces, `/`, `#` or `\`. Those characters structure the thread id, so refusing them keeps two chats from deriving the same thread. |
| `sender_name` | Optional, at most 128 characters, no control characters. |
| `text` | Not blank, at most 32 KiB. |
| `client_id` | Optional `/events` client id for the replies, same rule as the ids; default `channel-relay`. |
| `attachments` | Reserved. A non-empty list is refused, never silently dropped. |

The answer comes at once: `{ accepted: true, thread_id, request_id, client_id }`. A `message_id` already recorded on its thread answers `{ accepted: false, duplicate: true, thread_id }` and runs nothing, so a gateway may retry a delivery.

Each reply arrives on the caller's `GET /events?client_id=<client_id>` stream as:

```json
{
  "event": "channel_outbound",
  "client_id": "channel-relay",
  "thread_id": "channel:telegram/555/777",
  "request_id": "<from the ack>",
  "full_response": "<reply text>",
  "structured": {
    "kind": "channel_outbound",
    "channel": "telegram",
    "chat_id": "777",
    "thread_ts": null,
    "reply_to_message_id": "tg-1",
    "idempotency_key": "<per send>"
  }
}
```

Replies and progress use the compiled TinyChannels progressive contract; the
relay sink does not expose provider reactions.

## How a message runs

1. `params.rs` validates the message. The confidential `PrepareChannelIdentity` operation returns the canonical thread id, `channel:<channel>/<sender>/<chat>`, shared with the backend-relayed inbound path. A user cannot create such a thread through `threads_upsert`, which reserves `channel:`.
2. `store.rs` creates the thread in the workspace of the caller's config and appends the message (`user:<message_id>`). In a single-user core the process-wide channel persistence subscriber is told to skip this turn (`threads::store::claim_channel_turn`), so it is not mirrored under a second id; in SaaS that subscriber is not registered and the claim is a no-op.
3. `ops.rs` builds a one-message `ChannelRuntimeContext` from the caller's config and policy (`channels::runtime::build_channel_turn_parts` with `PromptToolDescs::Registered`, then `runtime_context`): the caller's tools, model and workspace, never the process's channel runtime. The pipeline's per-chat history is seeded from the thread's earlier rows. The turn runs in the background under the caller's scope (`spawn_scoped`).
4. `process_channel_message` runs it like any channel message: slash commands, approval replies, then the orchestrator over `agent.run_turn` under the `ExternalChannel` origin.
5. `channel.rs`'s `RelayChannel` turns each send into a `channel_outbound` event. `publish_web_channel_event` stamps the publishing context's tenant (`profile`, `agent`), and `/events` filters on it, so only the caller's stream carries it.
6. The texts sent are appended to the thread as one reply (`assistant:<message_id>`).

## Files

| File | Purpose |
| --- | --- |
| `mod.rs` | Module docs and re-exports |
| `params.rs` | `RelayInboundParams`, limits, validation, thread id and title |
| `channel.rs` | `RelayChannel`, the `Channel` sink that publishes `channel_outbound` |
| `store.rs` | Recording the inbound message and the reply; history from the thread |
| `ops.rs` | `channel_relay_inbound` and the turn runner |
| `schemas.rs` | The `channel.relay_inbound` controller |

Tests sit beside each file (`*_tests.rs`). `tests/saas_mode_e2e.rs::a_profile_holds_web_and_relayed_channel_threads` covers the SaaS path end to end.

## Limits

- Per-chat `/models` overrides set from a relayed chat last for that message only: the runtime context is rebuilt per message.
- The tool registry and prompt are built per message.
- In a single-user core with in-chat approvals, a parked approval is announced through the started channel runtime, which has no relay sink. In SaaS the gate never parks.
- Native per-profile listeners (a user's own bot token) are not served here; see the SaaS profiles plan.

## Further reading

- [Providers](../README.md)
- [Channels](../../README.md)

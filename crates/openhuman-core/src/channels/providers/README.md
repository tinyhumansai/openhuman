# Providers

Provider transports and construction live in [`vendor/tinychannels`](../../../../../vendor/tinychannels/) and execute inside its separately loaded TinyBus module. OpenHuman sends credential-hydrated configuration through `modules::channel_runtime`, receives scoped callbacks, and owns product policy and turn orchestration. The provider re-exports in [`mod.rs`](./mod.rs) are test-only compatibility fixtures.

The `channels` feature enables the module loader. It exposes Telegram, Discord, Slack, Signal, WhatsApp, iMessage, IRC, email and the other providers declared by the module. The local CLI REPL is OpenHuman orchestration.

## Relay

[`relay/`](./relay/README.md) is the host entry for messages a gateway relays from a hosted platform (`openhuman.channel_relay_inbound`). OpenHuman applies its dispatch policy and publishes replies as `channel_outbound` events; relay configuration, pairing and provider delivery execute through the module.

## Provider capabilities

The contract declares `remote_control`, `chat_approvals`, `progressive_edits` and `history_key_ignores_thread` in `tinychannels_bus::capabilities_for`. The host uses these flags to select product behavior:

- `routes.rs` dispatches remote-control commands through the module and supplies host callbacks for session operations.
- `runtime/dispatch/processor/approval.rs` scopes approval turns; `host/channel_events.rs` presents approval prompts and turn state.
- `bus/` orchestrates progressive replies through module-owned delivery handles.

Module requests carrying configuration, credentials or message content use confidential calls. Private runtime handles retain their scoped host callbacks until stop is acknowledged; cancelled or failed cleanup transfers to the process-lifetime reaper.

## Adding a provider

Implement provider transport, contract declarations and registration in the canonical TinyChannels repository first. Consume a released module artifact with verified digests, then extend OpenHuman's configuration and credential hydration where needed. Provider secrets outside `config.toml` are hydrated in `runtime/startup/credentials.rs` before the module receives configuration.

## Further reading

- [Parent module README](../README.md)
- [Messaging channels](../../../../../docs/gitbooks/en/features/channels.md)
- [tinychannels](../../../../../vendor/tinychannels/README.md)

# Providers

Implementations live in [`vendor/tinychannels/src/providers/`](../../../../../vendor/tinychannels/src/providers/), not here. [`mod.rs`](./mod.rs) re-exports each provider module from `tinychannels::providers`, which keeps the stable `crate::channels::providers::<name>` path, and via [`channels/mod.rs`](../mod.rs) the shorter `crate::channels::<name>` path. Provider construction is upstream too: `runtime/startup.rs` calls `tinychannels::build_channels` with a credential-hydrated config and the `channels::host` capability surface.

## Providers

| Provider        | Re-exported struct   | Feature gate                                             |
| --------------- | -------------------- | -------------------------------------------------------- |
| `dingtalk`      | `DingTalkChannel`    | `channels`                                               |
| `discord`       | `DiscordChannel`     | `channels`                                               |
| `email_channel` | `EmailChannel`       | `channels`                                               |
| `imessage`      | `IMessageChannel`    | `channels`                                               |
| `irc`           | `IrcChannel`         | `channels`                                               |
| `lark`          | `LarkChannel`        | `channels`                                               |
| `linq`          | `LinqChannel`        | `channels`                                               |
| `mattermost`    | `MattermostChannel`  | `channels`                                               |
| `qq`            | `QQChannel`          | `channels`                                               |
| `signal`        | `SignalChannel`      | `channels`                                               |
| `slack`         | `SlackChannel`       | `channels`                                               |
| `telegram`      | `TelegramChannel`    | `channels`                                               |
| `whatsapp`      | `WhatsAppChannel`    | `channels`                                               |
| `whatsapp_web`  | `WhatsAppWebChannel` | `whatsapp-web` (forwards to `tinychannels/whatsapp-web`) |
| `yuanbao`       | `YuanbaoChannel`     | `channels`                                               |

The local CLI REPL is OpenHuman orchestration and does not import a TinyChannels CLI provider.

## Relay

[`relay/`](./relay/README.md) is not a provider transport. It is the host entry for messages a gateway relays in from a hosted platform (`openhuman.channel_relay_inbound`), answered through the dispatch pipeline with replies published as `channel_outbound` events.

## Capabilities, not provider special cases

What a provider can do beyond send/receive is declared once, upstream, in `tinychannels_bus::capabilities_for` (`ChannelCapabilities`): `remote_control`, `chat_approvals`, `progressive_edits` and `history_key_ignores_thread`. The host reads those flags instead of comparing channel names:

- `routes.rs` offers `/status`, `/sessions`, `/new` and `/help` (`tinychannels::remote`) where `remote_control` is set, backed by `host/remote_control.rs`.
- `runtime/dispatch/processor/approval.rs` scopes turns in an `ApprovalChatContext` and intercepts `yes`/`no` replies where `chat_approvals` is set; `host/channel_events.rs::ChannelApprovalSurfaceSubscriber` sends the prompt (`tinychannels::approvals`).
- `host/channel_events.rs::ChannelTurnStateSubscriber` records busy state for `/status`.
- `bus/`'s progressive reply shows draft, thinking and filler bubbles only where `progressive_edits` is set.

Ported providers reach host capabilities (voice, approvals, conversation history, shutdown, event sink) through the `tinychannels::host::ProviderContext` built in `channels::host` instead of calling OpenHuman internals directly; see [`channels/host/mod.rs`](../host/mod.rs).

## Adding a provider

Provider transport, registration (`ChannelDefinition` metadata in `tinychannels::controllers`) and its capability entry belong upstream in [`vendor/tinychannels`](../../../../../vendor/tinychannels/). Once a provider exists there:

1. Add it to the `pub use tinychannels::providers::{...}` list in `providers/mod.rs`.
2. Add the matching `pub use providers::<name>` / `pub use <name>::<Name>Channel` pair to `channels/mod.rs`.
3. If the provider's secret lives outside `config.toml` (keyring or env), extend `hydrate_channel_credentials` in `runtime/startup/credentials.rs` the way `email` and `yuanbao` do; `tinychannels::build_channels` expects an already-hydrated config.

Do not reimplement provider transport logic, or branch on a provider name for behaviour a capability flag describes; add the flag upstream instead.

## Further reading

- [Parent module README](../README.md)
- [Messaging channels](../../../../../gitbooks/features/channels.md)
- [tinychannels](../../../../../vendor/tinychannels/README.md)

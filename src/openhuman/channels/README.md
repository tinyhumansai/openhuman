# Channels

Multi-platform messaging integration. Owns the `Channel` trait, per-provider connectors (Slack, Discord, Telegram, WhatsApp, IRC, Matrix, Signal, iMessage, Email, Lark, Mattermost, DingTalk, QQ, Linq, Web, CLI), the runtime supervisor that brings channels online, inbound dispatch into the agent loop, and proactive outbound delivery. Does NOT own the channel system prompt copy (lives in `context/channels_prompt.rs`) or per-channel credential storage (delegated to `credentials/`).

## Public surface

- `pub trait Channel` / `pub struct SendMessage` / `pub struct ChannelMessage` — `traits.rs:5-60` — provider contract for inbound + outbound messages.
- `pub struct ChannelDefinition` / `pub enum ChannelAuthMode` — `controllers/definitions.rs` (re-exported `mod.rs:59`) — declarative provider metadata.
- `pub fn start_channels` — `runtime/startup.rs` (re-exported `mod.rs:65`) — boot all enabled channels under the supervisor.
- `pub fn doctor_channels` — `commands.rs` — diagnose connectivity for the doctor CLI.
- `pub fn build_system_prompt` — re-exported from `crate::openhuman::context::channels_prompt`.
- Per-provider channel structs: `pub struct CliChannel`, `DingTalkChannel`, `DiscordChannel`, `EmailChannel`, `IMessageChannel`, `IrcChannel`, `LarkChannel`, `LinqChannel`, `MattermostChannel`, `QQChannel`, `SignalChannel`, `SlackChannel`, `TelegramChannel`, `WhatsAppChannel` — `providers/<name>.rs`. Cargo-feature-gated: `MatrixChannel` (`channel-matrix`), `WhatsAppWebChannel` (`whatsapp-web`).
- Stable `pub use providers::<name>` paths for every provider — `mod.rs:18-36`.
- RPC `channels.{list, describe, connect, disconnect, status, test, telegram_login_start, telegram_login_check, discord_link_start, discord_link_check, discord_list_guilds, discord_list_channels, discord_check_permissions, send_message, send_reaction, create_thread, update_thread, list_threads}` — `controllers/schemas.rs`.

## Calls into

- `src/openhuman/agent/` — inbound messages spawn or resume agent runs through `runtime/dispatch.rs`.
- `src/openhuman/credentials/` — per-channel auth tokens, refresh flow.
- `src/openhuman/config/schema/channels.rs` — runtime channel configuration.
- `src/openhuman/threads/` — thread state for platforms with native threading (Slack `thread_ts`).
- `src/openhuman/notifications/` — surface inbound deliveries to the UI.
- `src/openhuman/encryption/` — at-rest secret protection.
- `src/core/event_bus/` — emits `DomainEvent::Channel(*)`; `channels/bus.rs` registers `ChannelInboundSubscriber`.

## Called by

- `src/openhuman/threads/ops.rs` — thread lifecycle uses channel send paths.
- `src/openhuman/memory/conversations/bus.rs` — persists incoming channel messages as conversation memories.
- `src/openhuman/cron/bus.rs` — scheduled triggers can post via channels.
- `src/openhuman/config/schema/channels.rs` — config layer references channel types for validation.
- `src/core/all.rs` — controller registry wiring.

## Tests

- Unit: `bus_tests.rs`, `routes_tests.rs`, plus per-provider `*_tests.rs` (`email_channel_tests.rs`, `imessage_tests.rs`, `irc_tests.rs`, `lark_tests.rs`, `linq_tests.rs`, `matrix_tests.rs`, `mattermost_tests.rs`, `qq_tests.rs`, `signal_tests.rs`, `web_tests.rs`, `whatsapp_tests.rs`, `whatsapp_web_tests.rs`, `presentation_tests.rs`).
- Cross-channel integration tests: `tests/discord_integration.rs`, `tests/telegram_integration.rs`, `tests/runtime_dispatch.rs`, `tests/common.rs`.
- Telegram channel-level: `providers/telegram/channel_tests.rs`.
- Controller tests: `controllers/{definitions_tests,ops_tests,schemas_tests}.rs`.

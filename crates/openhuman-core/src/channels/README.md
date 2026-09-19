# Channels

Multi-platform messaging integration. Re-exports the `Channel` trait vocabulary, owns the host glue for every provider (Slack, Discord, Telegram, WhatsApp, WhatsApp Web, IRC, Signal, iMessage, Email, Lark, Mattermost, DingTalk, QQ, Linq, Yuanbao, and the local CLI REPL), the runtime supervisor that brings channels online, inbound dispatch into the agent loop, and proactive outbound delivery. Does NOT own the channel system prompt copy (lives in `agent/context/channels_prompt.rs`; `system_prompt.rs` here owns only when it is re-rendered), per-channel credential storage (delegated to `security/credentials/`), or provider transport implementations (vendored in `tinychannels`; see below).

## Feature gate

The domain lives behind the `channels` feature (default-ON, #4801), except two dependency-free carve-outs that always-on code reaches directly: `traits` (the `Channel`/`SendMessage` re-export, named by the always-on agent-harness interactive loop in `agent/session_host/`) and `cli` (`CliChannel`, the stdin/stdout REPL that loop drives in every build). Everything else — `providers`, `host`, `controllers`, `runtime`, `bus`, `proactive`, `commands`, `context`, `routes`, `relay_runtime`, `system_prompt`, the provider re-exports, `doctor_channels`, `start_channels`, `build_system_prompt` and the `test_support` re-export — is `#[cfg(feature = "channels")]`. The feature owns the optional `tinychannels` crate (`channels = ["dep:tinychannels", "tinychannels/email", "tinychannels/lark"]`); the always-on pins (config schema, the `DomainEvent` inbound envelope, security pairing) resolve through the unconditional `tinychannels-bus` contract crate instead. Measured, the gate sheds no transitive packages — its value is compile surface and binary size — and `voice` re-enables `dep:tinychannels` for `EmailChannel` alone. The full rationale is the comment above the `channels` feature in `crates/openhuman-core/Cargo.toml`.

## Public surface

- `pub use tinychannels_bus::{Channel, ChannelMessage, ChannelSendExt, SendMessage}` — `traits.rs` — the provider contract, sourced from the transport-free `tinychannels-bus` contract crate so it resolves even in `channels`-less builds.
- `ChannelDefinition` / `ChannelAuthMode` — re-exported from `tinychannels::controllers` through `controllers/definitions.rs` and `mod.rs` — declarative provider metadata.
- `pub fn start_channels` — `runtime/startup.rs` (re-exported from `mod.rs`) — boot all enabled channels under the supervisor.
- `pub fn doctor_channels` — `commands.rs` — diagnose connectivity for the doctor CLI.
- `pub fn build_system_prompt` — re-exported from `crate::agent::context::channels_prompt`.
- `pub(crate) enum ChannelSystemPrompt` — `system_prompt.rs` — the prompt a turn is seeded with: `fixed` (tests) or `refreshing` (production), which renders `build_system_prompt_with_identity` for the active agent definition.
- Per-provider channel structs re-exported from `providers/<name>.rs` (all thin `pub use tinychannels::providers::…` shims): `DingTalkChannel`, `DiscordChannel`, `EmailChannel`, `IMessageChannel`, `IrcChannel`, `LarkChannel`, `LinqChannel`, `MattermostChannel`, `QQChannel`, `SignalChannel`, `SlackChannel`, `TelegramChannel`, `WhatsAppChannel`, `YuanbaoChannel`. Cargo-feature-gated: `WhatsAppWebChannel` (`whatsapp-web`). `CliChannel` (`cli.rs`) is the one local implementation and is ungated.
- Stable `pub use providers::<name>` paths for every provider — `mod.rs`.
- RPC `channels.{list, describe, connect, disconnect, status, set_default, get_default, test, telegram_login_start, telegram_login_check, discord_link_start, discord_link_check, discord_list_guilds, discord_list_channels, discord_check_permissions, send_message, send_reaction, create_thread, update_thread, list_threads}` — `controllers/schemas.rs`.

## Submodule map

| Path | Purpose |
| --- | --- |
| `providers/` | Per-provider re-exports from `tinychannels`, plus Telegram's host-coupled glue ([README](providers/README.md)) |
| `host/` | `build_channel_host` / `build_provider_context`, the OpenHuman implementation of `tinychannels::host` that ported providers call back into (adapters in `host/adapters.rs`: `VoiceTranscriber`, `VoiceSynthesizer`, `CoreApprovalGate`, `ConversationHistoryStore`, `InferenceReactionGate`, `OpenHumanEventSink`, `CoreShutdownRegistry`, `ConfigAllowlistStore`) |
| `runtime/` | Startup, supervision, and the inbound dispatch loop into the agent ([README](runtime/README.md)) |
| `controllers/` | `channels.*` RPC namespace, `OpenHumanChannelBackend`, provider definitions ([README](controllers/README.md)) |
| `tests/` | Cross-channel integration test suite (`#[cfg(all(feature = "channels", test))]`) |

Flat files: `bus.rs` (`ChannelInboundSubscriber`, handles `DomainEvent::ChannelInboundMessage` published by the socket transport layer in `platform/socket/event_handlers.rs` and replies through the REST API; registered by `register_domain_subscribers` in `core/jsonrpc.rs`, not here), `cli.rs` (ungated `CliChannel`), `commands.rs` (`doctor_channels` and listener health classification), `context.rs` (`ChannelRuntimeContext`, per-sender conversation history, timeouts), `proactive.rs` (`ProactiveMessageSubscriber`, subscribes `DomainEvent::ProactiveMessageRequested` and delivers to the active channel), `relay_runtime.rs` (`relay_runtime_fronts_channel` / `send_outbound_intent` — the process-local relay-websocket transport handle controller sends go through when the relay fronts a channel), `routes.rs` (per-sender route overrides and runtime commands), `system_prompt.rs`, `traits.rs` (ungated `Channel`/`SendMessage` re-export).

## Calls into

- `crates/openhuman-core/src/agent/` — the agent turn is dispatched over `BUS.native()` to the `agent.run_turn` handler registered by `agent::bus::register_agent_handlers`, so `runtime/dispatch/` never imports the harness directly.
- `crates/openhuman-core/src/agent/context/channels_prompt.rs` — channel system prompt rendering, re-exported as `build_system_prompt`.
- `crates/openhuman-core/src/security/credentials/` — `AuthService` lookups for connect/disconnect and for secret hydration at startup (email password, Yuanbao app secret).
- `crates/openhuman-core/src/security/approval/` — `ApprovalGate` for the approval-reply intercept and `ApprovalChatContext` scoping of Telegram turns.
- `crates/openhuman-core/src/config/` — `Config` / `ChannelsConfig` (schema types come from `tinychannels_bus::config` via `config/schema/channels.rs`).
- `crates/openhuman-core/src/memory/conversations/` and `memory/guard` — conversation history persistence and the active memory guard.
- `crates/openhuman-core/src/api/rest.rs` — `BackendOAuthClient` for controller messaging ops and Telegram/Discord link flows.
- `crates/openhuman-core/src/web_chat/` — web-channel event publishing, session invalidation (`/new`), and the web surface subscribers registered at startup.
- `crates/openhuman-core/src/voice/` — STT/TTS behind the `host/` adapters.
- `crates/openhuman-core/src/core/bus.rs` and `core/events.rs` — the process-wide `BUS` and the `DomainEvent::Channel*` variants published from `runtime/dispatch/` and `runtime/supervision.rs`.
- `vendor/tinychannels/` — provider transports, `tinychannels::build_channels` (provider construction), `ChannelManager`/`ChannelBackend`, and the `tinychannels::host` capability boundary; `vendor/tinychannels/crates/tinychannels-bus` — the transport-free trait/type contract.

## Called by

- `crates/openhuman-core/src/core/all.rs` — registers `controllers::all_channels_registered_controllers()` under the `channels` feature gate.
- `crates/openhuman-core/src/core/runtime/services.rs` — `spawn_channels_service` calls `channels::start_channels(config)` unless `OPENHUMAN_DISABLE_CHANNEL_LISTENERS` is set or `has_listening_integrations()` is false.
- `crates/openhuman-core/src/core/jsonrpc.rs` — `bootstrap_core_runtime` subscribes `bus::ChannelInboundSubscriber` and calls `proactive::register_web_only_proactive_subscriber()`.
- `crates/openhuman-core/src/agent/session_host/` — the interactive loop drives the ungated `CliChannel`.
- `crates/openhuman-core/src/cron/bus.rs` — `CronDeliverySubscriber` is handed the started channel map by `runtime/startup.rs`; it names channels only through `tinychannels_bus`.

## Tests

- Unit, flat files: `bus_tests.rs`, `bus_inbound_thread_id_tests_tests.rs`, `cli_tests.rs`, `commands_tests.rs`, `context_tests.rs`, `proactive_tests.rs`, `relay_runtime_tests.rs`, `routes_tests.rs`, `system_prompt_tests.rs`, `traits_tests.rs` (`bus/` — `delivery.rs`, `draft.rs`, `filler.rs`, `progressive_ui.rs`, `streaming_state.rs`, `subscriber.rs`, `thinking.rs`, `thread_id.rs` — are source submodules, not tests; `bus_test_support_tests.rs` is a debug-build helper module).
- Cross-channel integration suite (`tests/`, see its module doc): `common.rs` fixtures plus `discord_integration`, `health`, `identity`, `memory`, `personality`, `prompt`, `runtime_dispatch`, `runtime_tool_calls`, `telegram_integration`.
- Host adapters: `host/host_tests.rs`.
- Provider host glue: `providers/telegram/{approval_surface_tests,bus_tests,remote_control_tests}.rs`.
- Controllers: `controllers/{backend_tests,ops_tests,schemas_tests}.rs` (+ `ops_connect_status_tests.rs`/`ops_yuanbao_email_tests.rs`), `controllers/ops/connect_email_config_tests_tests.rs`.
- Runtime: `runtime/{startup_tests,startup_email_secret_tests_tests,startup_yuanbao_secret_tests_tests,supervision_tests,dispatch_tests}.rs`, `runtime/dispatch/{mod_scoping_tests_tests,mod_approval_surface_gating_tests_tests,routing_connected_fallback_tests_tests}.rs`.

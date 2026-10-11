# channels

This domain connects OpenHuman to external messaging platforms: Slack, Discord, Telegram, WhatsApp (Cloud API and, behind a feature, WhatsApp Web), IRC, Signal, iMessage, Email, Lark, Mattermost, DingTalk, QQ, Linq, Yuanbao, and the local CLI REPL. A message that arrives on one of those platforms is turned into an agent turn, and the agent's reply is sent back to the same chat. The domain also owns the `channels.*` RPC namespace the settings UI uses to connect and test providers, and the subscriber that delivers proactive messages (briefings, cron output) to the user's chosen channel.

Most provider code is not here. Transports, provider construction, capability flags, remote-control commands, in-chat approval rendering and progressive reply choreography live in the vendored `tinychannels` crate ([`vendor/tinychannels/`](../../../../vendor/tinychannels/)). This folder is the host side: it supplies credentials, the agent, approvals, voice, persistence and events, and decides policy.

## How it works

There are two inbound paths. They look similar but run in different places.

### Path 1: native listeners (`start_channels`)

When the user has connected at least one listening integration, the core spawns the channel runtime. `spawn_channels_service` in [`core/runtime/services.rs`](../core/runtime/services.rs) captures the current channel session token, loads the config, checks `channels_config.has_listening_integrations()`, and calls `start_channels_with_session`. Setting `OPENHUMAN_DISABLE_CHANNEL_LISTENERS=1` skips the whole thing.

[`runtime/startup/start_channels.rs`](./runtime/startup/start_channels.rs) then does the boot work in this order:

1. Initialises the event bus and registers the subscribers a messaging core needs (health, thread persistence, Composio triggers, the web-chat surface subscribers, task sources) plus the native `agent.run_turn` handler. That handler must exist before the dispatch loop starts, because every inbound message is answered through it.
2. Resolves the chat model (`resolve_chat_workload` in `startup/chat_workload.rs` picks the managed cloud model or the user's per-workload provider), installs the live `SecurityPolicy`, builds the tool registry, and loads skills.
3. Builds the system prompt as a `ChannelSystemPrompt::refreshing` (see "System prompt" below). The fixed suffix is the XML tool-instruction block plus an advisory "Host access" section rendered by `startup/prompt.rs`.
4. Builds the `ChannelHost` capability surface (`host::build_channel_host`) and calls `tinychannels::build_channels` with a credential-hydrated config (`hydrate_channel_credentials` in `startup/credentials.rs` fills the email password and Yuanbao app secret from the keyring) and `RuntimeProxyClients` (the configured HTTP proxy).
5. Optionally connects the relay WebSocket runtime (`startup/relay.rs`) when `channels_config.relay` is configured.
6. Spawns one supervised listener per provider ([`runtime/supervision.rs`](./runtime/supervision.rs)), registers the cron delivery subscriber, the cron channel bridge, the proactive subscriber, and the turn-state and approval-surface subscribers from [`host/channel_events.rs`](./host/channel_events.rs).
7. Runs the dispatch loop until the session is cancelled.

```text
 provider listener (one per channel, supervised, backoff + jitter)
        |  ChannelMessage            relay websocket (optional)
        v                                   |  RuntimeChannelMessage
   mpsc(100) --> bridge task --> mpsc(100) <+
                                   |
                                   v
                 run_message_dispatch_loop
                 (bounded by compute_max_in_flight_messages)
                                   |
                                   v  one worker per message
                 process_channel_runtime_message (processor/turn.rs)
                   1. publish ChannelMessageReceived
                   2. slash command?  --> routes.rs, stop
                   3. yes/no for a parked approval? --> ApprovalGate, stop
                   4. typing indicator, ACK reaction
                   5. history = system prompt + per-sender history + msg
                   6. resolve_target_agent ("orchestrator") + tool scope
                   7. BUS.native().request("agent.run_turn", ...)
                      inside ApprovalChatContext when chat_approvals
                   8. draft updates / final reply on the channel
                   9. publish ChannelMessageProcessed
```

Per-sender history is kept in memory in `ChannelRuntimeContext` ([`context.rs`](./context.rs)), keyed by `conversation_history_key` (channel, sender, reply target, thread). When a turn fails with a context-window overflow, the processor compacts that sender's history and asks the user to resend. Turns have a timeout (`message_timeout_secs`). Channel turns do no per-turn memory recall or autosave: memory reaches a session through `context.md`, injected by the session host, and committed turns are ingested from the `ConversationTurnCommitted` event.

A channel can instead be bound to a host-registered agent with `config.agent.channel_agents` (channel name to agent id): `runtime/dispatch/host_agent/` then builds that agent's session and runs the turn as it, under the channel's `ExternalChannel` origin capped at read-only, and refuses the message when the bound agent is missing. See [runtime/README.md](runtime/README.md#channels-bound-to-a-host-agent).

The listener supervisor reports through `OpenHumanListenerObserver`, which publishes `DomainEvent::ChannelConnected`, `ChannelDisconnected` and `HealthRestarted`. The full runtime walk-through, including account lifetime and logout, is in [runtime/README.md](runtime/README.md).

### Path 2: backend-relayed inbound (`ChannelInboundSubscriber`)

The desktop app usually has no native listeners. Messages for hosted bots arrive over the backend socket instead: [`platform/socket/event_handlers.rs`](../platform/socket/event_handlers.rs) publishes `DomainEvent::ChannelInboundMessage`. `bus::ChannelInboundSubscriber` ([`bus/subscriber.rs`](./bus/subscriber.rs)) handles it. It is registered on the always-on boot path by `register_domain_subscribers` in [`core/runtime/subscribers.rs`](../core/runtime/subscribers.rs), not by `start_channels`, so it works on a core that never starts listeners.

The subscriber derives a thread id from (channel, sender, reply target, thread) with `derive_inbound_thread_id`, starts a turn through `web_chat::start_chat`, and watches the web-channel event stream for that request. The reply is shown progressively with `tinychannels::delivery::progressive::ProgressiveReply`: a draft that is edited as text streams in, typing refreshes, and filler bubbles on slow turns, then a finalize. The transport underneath is `BackendProgressiveSender` ([`bus/delivery.rs`](./bus/delivery.rs)), which sends through `BackendClient` with idempotency keys and maps typed `BackendApiError`s (`ChannelEditUnsupported`, `MessageNotFound`) to the recoveries the progressive driver expects.

```text
 backend socket --> DomainEvent::ChannelInboundMessage
                          |
                          v
               ChannelInboundSubscriber
                          |  web_chat::start_chat(thread_id, ...)
                          v
               web channel events --> ProgressiveReply
                                          |
                                          v
                               BackendProgressiveSender --> backend REST
```

### Path 3: gateway-relayed inbound (`channel_relay_inbound`)

A gateway that owns a hosted platform's webhook (the SaaS cloud gateway for Telegram, iMessage and Discord) relays each message in as its user with `openhuman.channel_relay_inbound` ([`providers/relay/`](./providers/relay/README.md)). The message is recorded on the caller's `channel:<channel>/<sender>/<chat>` thread (`derive_inbound_thread_id`), then runs through the Path 1 pipeline (`process_channel_message`, `ExternalChannel` origin) with a runtime context built from the caller's own config. Replies go to `RelayChannel`, which publishes them as `channel_outbound` events on the caller's `/events` stream for the gateway to deliver, and are recorded on the thread.

### Outbound: proactive messages

[`proactive.rs`](./proactive.rs) subscribes to `DomainEvent::ProactiveMessageRequested`. Every proactive message goes to the web channel (`web_chat::publish_web_channel_event`). If the user has set an active external channel and that channel is in the started channel map, it is sent there too. The always-on boot registers a web-only variant (`register_web_only_proactive_subscriber`); the full runtime registers one with the real channel map and publishes its active-channel handle, so `channels.set_default` (through `set_runtime_active_channel`) takes effect without a restart.

### Slash commands and remote control

[`routes.rs`](./routes.rs) intercepts messages starting with `/` before any turn runs. Portable commands (`/models`, `/providers` and the per-sender route override they set) come from `tinychannels::routes`. On providers with the `remote_control` capability, `/status`, `/sessions`, `/new` and `/help` are parsed and executed by `tinychannels::remote`, with [`host/remote_control.rs`](./host/remote_control.rs) (`RuntimeRemoteControl`) answering from the runtime context, the thread store and the web-chat session cache.

### In-chat approvals

When a tool call needs approval on a channel whose provider has `chat_approvals`, three pieces cooperate. The dispatch loop runs the turn inside an `ApprovalChatContext` whose thread id is the history key. `ChannelApprovalSurfaceSubscriber` (`host/channel_events.rs`) turns `ApprovalRequested` events into a prompt in that chat through `tinychannels::approvals::ApprovalSurface`. When the user replies yes or no, `try_route_approval_reply` ([`runtime/dispatch/processor/approval.rs`](./runtime/dispatch/processor/approval.rs)) hands it to `ApprovalGate::decide` instead of starting a new turn. Any other text runs as a fresh turn, which cancels the parked call. Providers without the capability (email, CLI, webhooks) keep the "no chat context, allow" behaviour, since a prompt there would only time out.

### System prompt

[`system_prompt.rs`](./system_prompt.rs) owns when the prompt is rendered, not its text. `ChannelSystemPrompt` has two variants. `Fixed` is a literal used by tests. `Refreshing` keeps the render inputs (`ChannelPromptInputs`: tool descriptions, skills, model, the fixed suffix) and re-renders only when the identity fingerprint changes. The fingerprint is a hash of `(mtime, len)` for the workspace-root `SOUL.md` and `IDENTITY.md`, so a steady-state message costs two `stat` calls and gets the same `Arc<String>` back, which keeps the provider prefix cache warm. The render itself is `build_system_prompt_with_identity` from [`agent/context/channels_prompt.rs`](../agent/context/channels_prompt.rs), with the project context (identity files) placed after the tool schemas and access block.

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](./mod.rs) | Module declarations, the feature-gate split, stable `channels::<provider>` paths and the public re-exports. |
| [`traits.rs`](./traits.rs) | Ungated re-export of `Channel`, `ChannelMessage`, `ChannelSendExt`, `SendMessage` from `tinychannels-bus`. |
| [`contract_schema.rs`](./contract_schema.rs) | Ungated conversion of `tinychannels-bus` controller schemas into core `ControllerSchema`s; shared with `openhuman-tinyhumans`. |
| `providers/` | One-line re-exports of each provider module from `tinychannels::providers`. See [providers/README.md](providers/README.md) for the provider table and how to add one. |
| `host/` | The OpenHuman implementation of the `tinychannels::host` boundary: `build_channel_host` and its adapters (`adapters.rs`), the approval-surface and turn-state subscribers (`channel_events.rs`), and the remote-control host (`remote_control.rs`). |
| `runtime/` | Startup, listener supervision, the account-lifetime session, and the dispatch pipeline. See [runtime/README.md](runtime/README.md). |
| `controllers/` | The `channels.*` RPC namespace and `OpenHumanChannelBackend`. See [controllers/README.md](controllers/README.md). |
| [`bus.rs`](./bus.rs), `bus/` | `ChannelInboundSubscriber` (`subscriber.rs`), the backend progressive sender (`delivery.rs`), and inbound identity derived directly through `tinychannels::channel`. |
| `context.rs` | `ChannelRuntimeContext`, per-sender history maps, history compaction, and timeout/backoff constants re-exported from `tinychannels::context`. |
| `routes.rs` | Slash-command parsing and per-sender route selection (provider and model overrides). |
| `proactive.rs` | `ProactiveMessageSubscriber` and the active-channel handle. |
| `system_prompt.rs` | `ChannelSystemPrompt`, `ChannelPromptInputs`, identity fingerprinting. |
| [`commands.rs`](./commands.rs) | `doctor_channels`: builds channels from the hydrated config and prints `tinychannels::runtime::check_channels_health` results. |
| `tests/` | Cross-channel integration suite (see Tests). |

## Key types and entry points

- `start_channels` / `start_channels_with_session` (`runtime/startup/start_channels.rs`) boot the native runtime. The `_with_session` form takes a `CancellationToken` captured before config loading, which is what `spawn_channels_service` uses.
- `session::channel_session` and `session::invalidate_channel_session` ([`runtime/session.rs`](./runtime/session.rs)) hold the process's single `tinychannels::runtime::ChannelSession`. Logout calls `invalidate_channel_session` from [`security/credentials/ops/credential.rs`](../security/credentials/ops/credential.rs), which cancels the running runtime and everything it owns.
- `ChannelRuntimeContext` (`context.rs`) is the shared state every dispatch worker reads: channel map, tools, prompt, model, histories, route overrides, timeouts, and the full `Config` in production.
- `process_channel_runtime_message` and `run_message_dispatch_loop` ([`runtime/dispatch/processor/`](./runtime/dispatch/processor/)) are the per-message pipeline and its bounded loop.
- `resolve_target_agent` ([`runtime/dispatch/routing.rs`](./runtime/dispatch/routing.rs)) routes every channel turn to the `orchestrator` agent and builds its visible tool set, falling back to `AgentScoping::unscoped()` when the registry or definition is missing.
- `build_channel_host` ([`host/mod.rs`](./host/mod.rs)) assembles the `ChannelHost` from `CoreShutdownRegistry`, `VoiceTranscriber`, `VoiceSynthesizer`, `CoreApprovalGate`, `ConversationHistoryStore`, `OpenHumanEventSink` and `ConfigAllowlistStore`. Reactions, turn dispatch, the run ledger and pairing are left unset.
- `ChannelInboundSubscriber` (`bus/subscriber.rs`) is the backend-relayed inbound path.
- `ProactiveMessageSubscriber` (`proactive.rs`) is outbound proactive delivery.
- `ChannelSystemPrompt` (`system_prompt.rs`) is the seed prompt for every turn.
- `OpenHumanChannelBackend` ([`controllers/backend.rs`](./controllers/backend.rs)) implements `tinychannels::ChannelBackend` for the RPC handlers.
- `ChannelDefinition` and `ChannelAuthMode` are defined by `tinychannels_bus::controllers` for declarative provider metadata.
- `build_system_prompt` is a re-export of `agent::context::channels_prompt::build_system_prompt`.

## RPC / CLI surface

The `channels` namespace is registered from `controllers::all_channels_registered_controllers()` in [`core/all.rs`](../core/all.rs), under `DomainGroup::Channels` and behind the `channels` feature. Schemas come from the `tinychannels-bus` contract.

- Provider metadata: `channels.list`, `channels.describe`.
- Connection lifecycle: `channels.connect`, `channels.disconnect` (optionally forgetting the channel's stored conversations), `channels.status`, `channels.test`.
- Default channel for proactive delivery: `channels.set_default`, `channels.get_default`.
- Discord: `channels.discord_list_guilds`, `channels.discord_list_channels`, `channels.discord_check_permissions`.
- Messaging through the backend: `channels.send_message`, `channels.send_reaction`, `channels.create_thread`, `channels.update_thread`, `channels.list_threads`.

The managed-bot link methods `channels.telegram_login_start`, `telegram_login_check`, `discord_link_start` and `discord_link_check` are part of the contract but are not registered here. They are listed in `HOSTED_CHANNEL_FUNCTIONS` ([`controllers/schemas.rs`](./controllers/schemas.rs)) and served by `openhuman-tinyhumans` (`hosted/channel_link/`), which converts their schemas with `channels::contract_schema::contract_controller_schema`.

The in-app web chat (RPC namespace `channel`, singular) is a separate domain in `web_chat/`.

## Boundaries

- Provider transports, provider construction (`tinychannels::build_channels`), `ChannelManager`/`ChannelBackend`, per-provider `ChannelCapabilities`, remote-control commands (`tinychannels::remote`), approval prompts (`tinychannels::approvals`), progressive delivery, the relay transport, connect-form parsing and health checks all belong to `tinychannels` (`vendor/tinychannels/`, repo `tinyhumansai/tinychannels`). The transport-free trait and type contract is [`vendor/tinychannels/crates/tinychannels-bus`](../../../../vendor/tinychannels/crates/tinychannels-bus/). Fix provider bugs there, not with a host-side branch on a channel name; add a capability flag upstream instead.
- The channel system prompt text is in `agent/context/channels_prompt.rs`.
- The agent turn itself is run by the `agent.run_turn` native handler registered by `agent::bus::register_agent_handlers`. The dispatch code never imports the harness directly.
- Credential storage is [`security/credentials/`](../security/credentials/) (`AuthService`); the approval gate and `ApprovalChatContext` are [`security/approval/`](../security/approval/).
- Config schema types come from `tinychannels_bus::config` through [`config/schema/channels.rs`](../config/schema/channels.rs).
- Conversation memory ingestion and `forget_channel` are in `memory/conversations/`.
- Backend REST calls go through `backend::BackendClient`; Telegram and Discord link flows that need a TinyHumans account live in `openhuman-tinyhumans`.
- Discord bot guild/channel discovery and permission checks call the released TinyChannels module through the confidential `ChannelController` member. The host resolves stored credentials and proxy scope, and the bus contract preserves the existing RPC response fields.
- `channels.test` delegates field validation and email IMAP verification to the same confidential controller member. It never persists credentials or configuration, and an unavailable module fails the test instead of reporting local validation as success.
- Speech-to-text and text-to-speech are the `voice/` domain, reached through the `host/` adapters.
- Web-chat turns, the web-channel event stream, and the surface subscribers are `web_chat/`.
- Cron delivery is [`cron/bus.rs`](../cron/bus.rs) (`CronDeliverySubscriber`) and [`cron/channel_bridge.rs`](../cron/channel_bridge.rs); `start_channels` hands them the started channel map.

## Gotchas

- The default `channels` feature gates provider orchestration and enables the module loader. Serialized config, messages, and controller declarations use the unconditional `tinychannels-bus` contract. Legacy provider traits and runtime helpers remain linked as explicit migration exceptions. The CLI REPL is host-owned orchestration. `WhatsAppWebChannel` additionally needs the `whatsapp-web` feature.
- `start_channels` is skipped on cores with no listening integrations, which includes most desktop installs. Anything every core needs (the `ChannelInboundSubscriber`, the web-only proactive subscriber, tool-timeout seeding, self-improvement subscribers, the flows trigger subscriber) is registered in `core/runtime/subscribers.rs`, not in `start_channels`. Put new always-needed wiring there.
- Several registrations in `start_channels` overlap with `bootstrap_core_runtime` (agent handlers, the agent definition registry). They are written to be idempotent; keep them that way.
- The channel session token must be captured before the first `await` on config loading. Otherwise a logout during startup leaves a listener running for the old account.
- The system prompt is frozen for the process except for the identity files. Tools, skills, model and security posture still need a restart to reach channel turns.
- History keys include the sender and thread, not only the channel. Keying on the channel alone would merge different participants of a shared Slack or Discord channel into one session.

## Tests

Unit tests sit beside their modules as `*_tests.rs` files (for example [`bus_tests.rs`](./bus_tests.rs), [`routes_tests.rs`](./routes_tests.rs), [`host/host_tests.rs`](./host/host_tests.rs), [`controllers/ops_tests.rs`](./controllers/ops_tests.rs), [`runtime/startup_tests.rs`](./runtime/startup_tests.rs)). The cross-channel integration suite in `tests/` (declared as `#[cfg(all(feature = "channels", test))] mod tests` in `mod.rs`) covers end-to-end dispatch, tool calls, health restarts, prompt assembly, and the Discord and Telegram flows; its `mod.rs` describes each file. Behaviour owned by `tinychannels` is tested upstream.

```bash
cargo test -p openhuman channels::
pnpm debug rust channels::
```

## Further reading

- [Parent module README](../../README.md)
- [Messaging channels](../../../../gitbooks/features/channels.md)
- [Deep architecture reference](../../../../gitbooks/developing/architecture.md)
- [tinychannels](../../../../vendor/tinychannels/README.md)

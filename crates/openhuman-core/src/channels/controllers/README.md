# Controllers

Owns the `channels.*` RPC namespace: provider metadata, connect/disconnect lifecycle, status, messaging, and per-provider auth flows (Telegram login, Discord OAuth link).

## Files

| Path             | Purpose                                                                                                                                                                                                                                                                                                                                                                                                                         |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`backend.rs`](./backend.rs)     | `OpenHumanChannelBackend`, the OpenHuman implementation of `tinychannels::ChannelBackend`. `tinychannels::ChannelManager` calls back into it, and each method forwards to the matching `ops` function. `send_outbound_intent` first tries `tinychannels::relay::send_outbound_intent` when `relay_runtime_fronts_channel` says the relay-websocket transport fronts the channel, then falls back to `ops::channel_send_message` |
| [`definitions.rs`](./definitions.rs) | Re-exports provider metadata (`ChannelDefinition`, `ChannelAuthMode`, `ChannelCapability`, `AuthModeSpec`, `FieldRequirement`, `all_channel_definitions`, `find_channel_definition`) from `tinychannels::controllers`                                                                                                                                                                                                           |
| `ops/`           | Business logic behind each RPC handler, grouped by concern (connect, discord, messaging)                                                                                                                                                                                                                                                                                                                                        |
| [`schemas.rs`](./schemas.rs)     | `all_registered_controllers` / `all_controller_schemas` and the thin RPC handlers: deserialize params, build a `ChannelManager<OpenHumanChannelBackend>` over the loaded config, call the manager, and shape the `Outcome`. Field schemas come from `tinychannels::controllers::channel_controller_schema`, converted by `from_channel_controller_schema`                                                                       |

## RPC surface

`channels.{list, describe, connect, disconnect, status, set_default, get_default, test, discord_list_guilds, discord_list_channels, discord_check_permissions, send_message, send_reaction, create_thread, update_thread, list_threads}`: declared in `schemas.rs::all_registered_controllers`. `list` and `describe` use a backend-less `ChannelManager::new(ChannelsConfig::default(), ())`; every other handler goes through `OpenHumanChannelBackend`. The managed-bot link methods of the contract (`telegram_login_start`, `telegram_login_check`, `discord_link_start`, `discord_link_check`, listed in `HOSTED_CHANNEL_FUNCTIONS`) need a TinyHumans account and are served by `openhuman-tinyhumans` (`hosted::channel_link`); `OpenHumanChannelBackend` implements them only to answer `BACKEND_UNAVAILABLE:`.

## `ops/`

| Path                                                                                                                                                   | Purpose                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| ------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `connect.rs` (+ `connect/`: `connect_channel.rs`, `disconnect.rs`, `email.rs`, `memory.rs`, `shared.rs`, `status.rs`, `test_channel.rs`) | `connect_channel`, `disconnect_channel`, `channel_status`, `test_channel`, `get_default_channel`/`set_default_channel`, clearing a disconnected channel's stored conversations through `memory::conversations::forget_channel` (`memory.rs`), email config persistence (`email.rs`; building and verifying email and Yuanbao configs is `tinychannels::controllers::build_email_config`, `tinychannels::providers::verify_email_credentials` and `tinychannels::providers::yuanbao::connect`), `connected_channel_slugs`, `merge_listener_health` (`pub(crate)`, re-exported from [`ops/mod.rs`](./ops/mod.rs) under `#[cfg(test)]`) |
| `discord.rs`                                                                                                                                           | Discord OAuth link flow and guild/channel/permission listing                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `messaging.rs`                                                                                                                                         | `channel_send_message`, `channel_send_reaction`, `channel_create_thread`, `channel_update_thread`, `channel_list_threads`: all call the TinyHumans backend REST API (`crate::backend::BackendClient`); the only path that reaches the relay transport registry (`tinychannels::relay::runtime`) is `OpenHumanChannelBackend::send_outbound_intent` in `backend.rs`                                                                                                                                                                 |
| `types.rs`                                                                                                                                             | Re-exports of `tinychannels_bus::controllers` result/snapshot types used by the ops layer                                                                                                                                                                                                                                                                                                                                                                                                                                               |

`connected_channel_slugs` (from `connect.rs`) is re-exported at `channels::controllers::connected_channel_slugs` for callers outside the controller registry; the `ops/mod.rs` comment cites the welcome agent's onboarding snapshot, but nothing outside [`channels/controllers/`](./) calls it today. `types.rs` re-exports the `tinychannels_bus::controllers` result types (`ChannelStatusEntry`, `ChannelSendMessageResult`, and others) rather than defining its own.

## Wiring

[`crates/openhuman-core/src/core/all.rs`](../../core/all.rs) pushes `crate::channels::controllers::all_channels_registered_controllers()` under `DomainGroup::Channels` behind `#[cfg(feature = "channels")]`. That list also carries the hosted-channel relay's `channel.relay_inbound` ([`providers/relay/`](../providers/relay/README.md)). The in-app web chat (`web_chat`, RPC namespace `channel`) is pushed under the same `DomainGroup::Channels` just above it and is not gated.

## Tests

- [`backend_tests.rs`](./backend_tests.rs).
- [`ops_tests.rs`](./ops_tests.rs) (declared from `ops/mod.rs`; pulls in [`ops_connect_status_tests.rs`](./ops_connect_status_tests.rs) and [`ops_yuanbao_email_tests.rs`](./ops_yuanbao_email_tests.rs)).
- [`schemas_tests.rs`](./schemas_tests.rs).

## Further reading

- [Parent module README](../README.md)
- [Messaging channels](../../../../../gitbooks/features/channels.md)
- [tinychannels](../../../../../vendor/tinychannels/README.md)

Credential testing (`test_channel`) delegates structural validation and IMAP verification through the released module’s confidential controller member. Config persistence and credential custody remain in OpenHuman.

Connection preparation uses TinyChannels' confidential `ChannelController`
`PrepareConnect` operation. The module validates credentials, prepares provider
defaults and verifies native login before returning a typed configuration update.
OpenHuman persists only the requested provider, retains encrypted credential
custody, and removes email passwords and Yuanbao application secrets from TOML.
An unavailable module fails both `connect` and `test`; neither operation executes
provider validation locally. Host persistence tests use recorded contract
results; provider verification/defaulting regressions live in TinyChannels.

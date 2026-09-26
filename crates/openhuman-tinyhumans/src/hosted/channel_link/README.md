# channel_link (hosted)

Links the managed TinyHumans Telegram and Discord bots to the user's
TinyHumans account, on the SDK's typed `auth()` client.

| Method | Backend call |
| --- | --- |
| `auth_create_channel_link_token` (`channel`) | `POST /auth/channels/{channel}/link-token` |
| `channels_telegram_login_start` | link token, then a `t.me/<bot>?start=<token>` deep link |
| `channels_telegram_login_check` (`linkToken`) | `GET /auth/me`, looks for `telegramId`; stores `channel:telegram:managed_dm` |
| `channels_discord_link_start` | link token, then `!start <token>` instructions |
| `channels_discord_link_check` (`linkToken`) | `GET /auth/me`, looks for `discordId`; stores `channel:discord:managed_dm` |

The `channels_*` schemas come from the `tinychannels-bus` contract, converted
by the core's always-compiled `channels::contract_schema`. They share the `channels` / `auth` namespaces with the
core, which keeps the rest of the channel controllers (connect, status, bot-token
Discord discovery, messaging) and the credential controllers. The core's
`OpenHumanChannelBackend` still implements the four contract methods, but only
to answer `BACKEND_UNAVAILABLE:`, since nothing in the core dispatches them any more.

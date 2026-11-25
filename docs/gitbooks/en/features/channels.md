---
description: >-
  Messaging apps OpenHuman talks back to you on: how messages reach the agent,
  how replies and proactive messages go out, and how each channel signs in.
icon: messages-square
---

# Messaging channels

A channel is a messaging app the agent uses to talk to you. You message the agent on a platform you already use, and it replies there. A channel is the opposite of an [integration](integrations/README.md). An integration is mostly something the agent reads from, such as your inbox, calendar or CRM. A channel is a two-way conversation.

Channel providers normalize platform messages into typed envelopes and send
typed replies through the configured provider. OpenHuman keeps account policy,
authorization, agent execution, and durable conversation sessions in the host;
provider transport runs in the TinyChannels v2 module behind its bus contract.
The host communicates with the module through typed TinyBus requests and
callbacks: it does not link provider implementations. The module prepares
provider identity and ordered message parts and owns the canonical typed
channel and web-event contracts; OpenHuman checks access, selects the agent,
and binds the message to its durable session before running a turn. Generic
turn admission and queue mechanics come from TinyAgents.

## What a channel does

A channel handles two directions.

**Inbound.** The provider module normalizes the message, address, sender, and
ordered content into a typed envelope. The OpenHuman host applies authorization
and routing policy, then starts or resumes the durable conversation session.
Some platforms support `/models` and `/model` to switch the model for that
sender's session. Telegram also supports remote-control commands.

**Outbound.** Typed reply content goes through the provider module to the
original reply target, using a thread address when the platform supports it.
Agent progress is projected through the shared TinyChannels web-output module
for Web chat, TUI, and relayed web events; a gateway may also deliver its
provider-specific terminal event. A channel can also send proactively, with no
incoming message, when a [trigger](integrations/triggers.md) or cron job fires.
A channel receives proactive messages only if it has a default delivery
target. Channels without one are skipped.

Where the platform allows it, channels can show a typing indicator, stream
draft updates, post threaded replies, send supported media or voice notes, and
add emoji reactions. These are provider-specific capabilities: the module
reports what each configured provider supports, and OpenHuman does not emulate
an unsupported operation.

## Supported channels

OpenHuman has 15 channel providers. 14 are built into the shipped desktop app. WhatsApp Web is provided by the TinyChannels module and is not enabled in the shipped app. The in-app Web chat is built in and is not a provider. A separate `cli` channel serves the `openhuman-core` terminal binary. Eight channels have a setup flow in the app: Telegram, Discord, Web, iMessage, Lark/Feishu, DingTalk, Email and 元宝 (Yuanbao). You turn on the rest by hand in `config.toml`.

| Channel        | Direction     | Inbound transport            | Credentials                                                    | In Settings UI |
| -------------- | ------------- | ---------------------------- | -------------------------------------------------------------- | -------------- |
| Telegram       | Two-way       | Bot API long-poll            | Connect via OpenHuman (managed DM) or your own BotFather token | Yes            |
| Discord        | Two-way       | Gateway                      | Your own bot token, OAuth install, or managed account link     | Yes            |
| Web            | Two-way       | In-app                       | Built in, no setup (local)                                     | Yes            |
| iMessage       | Two-way       | macOS Messages (AppleScript) | Local only, no credentials (needs Full Disk Access)            | Yes            |
| Lark / Feishu  | Two-way       | WebSocket or webhook         | Your own app id and secret                                     | Yes            |
| DingTalk       | Two-way       | Stream Mode WebSocket        | Your own client id and secret                                  | Yes            |
| 元宝 (Yuanbao) | Two-way       | WebSocket                    | Your own AppID and AppSecret                                   | Yes            |
| Slack          | Two-way       | Events/socket                | Your own bot token                                             | `config.toml`  |
| WhatsApp       | Two-way       | Meta Cloud webhook           | Your own access token                                          | `config.toml`  |
| IRC            | Two-way       | Persistent socket            | Your own server and nick                                       | `config.toml`  |
| Signal         | Two-way       | signal-cli REST events       | Your own linked signal-cli account                             | `config.toml`  |
| Mattermost     | Two-way       | REST API polling             | Your own bot token                                             | `config.toml`  |
| QQ             | Two-way       | WebSocket                    | Your own bot credentials                                       | `config.toml`  |
| Linq           | Two-way (SMS) | Webhook                      | Your own API token                                             | `config.toml`  |
| Email          | Two-way       | IMAP IDLE and SMTP           | Your own mailbox credentials                                   | Yes            |

WhatsApp also has an experimental peer-to-peer variant in the TinyChannels module. Channels that use a webhook receive messages by HTTP push, so they need an HTTPS endpoint that the provider can reach.

Telegram supports typing indicators and live draft updates. Approval
interactions depend on the provider's declared capabilities and configured
channel surface. Discord adds native threaded replies. Media, voice, and
addressing support also varies by provider and configured capabilities. Web
supports rich text and stays entirely local.

### Email

Email is a native, self-hosted connector with no third-party broker. Inbound mail arrives over IMAP IDLE push, so new mail reaches the agent in seconds. The connection refreshes about every 29 minutes, as the RFC requires. Replies go out over SMTP with full attachment support, from your own address on any provider you configure.

An `allowed_senders` list controls who can reach the agent by email. Set it to the addresses you trust. In `config.toml`, an empty list denies everyone. The Connections UI is different: a blank field becomes `["*"]`, which allows any sender. Don't leave it blank if strangers should not be able to prompt your agent.

## Credential modes

Channels sign in one of three ways.

- **Connect via OpenHuman (managed).** A one-click encrypted connection brokered by the OpenHuman backend. It covers Telegram (message the managed bot directly) and Discord (link your account or install through OAuth). No tokens are stored on your machine.
- **Your own credentials.** You supply a bot token, API key and secret, or app credentials. Telegram (BotFather token), Discord, Slack, WhatsApp, Lark/Feishu, DingTalk, Yuanbao, Signal, Mattermost, QQ, Linq, IRC and Email all support this. You get the most control, and you own the platform account, rate limits and any webhook endpoint.
- **Local, no credentials.** Web chat and iMessage need no tokens. Web runs inside the desktop app. iMessage drives the local macOS Messages app through AppleScript, so grant Full Disk Access. Both keep messages on your machine.

Secrets for any mode go through OpenHuman's credential layer and are encrypted at rest (see [Privacy and security](privacy-and-security.md)). Channels managed in the UI never write secrets to `config.toml` in plaintext.

## Where to connect a channel

Set up channels under **Connections > Channels** in the left sidebar, not under Settings. Open that tab, pick a platform tile and follow its setup card:

- **Discord.** Choose Connect via OpenHuman (link your account or install the bot through OAuth), or paste your own bot token.
- **Telegram.** Message the managed OpenHuman bot to link, or paste a BotFather token.

Slack is connected as an app under **Connections > OAuth** so the agent can read and act in Slack. It is not set up as a talk-back channel in the Channels tab.

## Choosing the default channel

In **Connections > Channels** you can pick the active route, which is the channel used for proactive messages with no recipient, such as cron jobs and triggers. The default is the in-app Web chat. A new default takes effect immediately, with no restart, and the panel shows which channel is active. Replies to inbound messages always go back on the channel they came from, whatever the default is.

## See also

- [Integrations](integrations/README.md): the catalog the agent pulls context from.
- [Triggers](integrations/triggers.md): live events that fire proactive channel messages.
- [Privacy and security](privacy-and-security.md): where credentials live and the backend boundary.
- [OS keyring and secret storage](os-keyring-and-secret-storage.md): at-rest protection for channel secrets.

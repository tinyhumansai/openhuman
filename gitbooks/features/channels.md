---
description: >-
  Messaging platforms OpenHuman talks back to you on: inbound dispatch into the
  agent loop, outbound replies and proactive delivery, and per-channel
  credentials.
icon: messages-square
---

# Messaging Channels

A **channel** is a messaging platform OpenHuman uses to _talk back_ to you. This is the mirror image of an [integration](integrations/README.md): an integration is mostly a source the agent _reads from_ (your inbox, your calendar, your CRM), while a channel is a two-way conversation surface. You message the agent on a platform you already use, and the agent replies there.

Under the hood every channel implements one small Rust contract (a `send` path for outbound messages and a `listen` path for inbound ones), so the same agent loop serves Telegram, Discord, the built-in web chat, and a dozen others without per-platform branching in the core.

***

## What a channel does

Each channel does two things:

* **Inbound**: when a message arrives, the channel normalizes it into a `ChannelMessage` (sender, reply target, content, optional thread id) and hands it to the dispatch loop. Dispatch spawns or resumes an agent run, scopes its tools, and the agent works the request. Some platforms support a `/models` and `/model` command to switch the model for that sender's session; Telegram additionally supports remote-control commands.
* **Outbound**: the agent's response is sent back through the same channel to your `reply_target`, threaded when the platform supports it. Channels can also deliver **proactively** (no incoming message to reply to) when fired by a [trigger](integrations/triggers.md), a cron job, or the [subconscious loop](subconscious.md). A channel only receives proactive sends if it advertises a default delivery target; channels without one are skipped rather than posted to an empty recipient.

Channels that support it can show a typing indicator, stream progressive **draft updates**, post **threaded replies**, and add **emoji reactions**. Capabilities are declared per channel, not assumed.

***

## Supported channels

OpenHuman ships **18 channel provider modules** (16 built by default plus two behind Cargo feature flags), of which **17 are real messaging platforms**. The remaining one, `presentation`, is an internal response-rendering helper for the web chat, not a platform you connect to. A separate `cli` channel serves the `openhuman-core` terminal binary. Seven channels are exposed in the Settings UI; the rest are enabled through `config.toml`.

| Channel | Direction | Inbound transport | Credential mode | In Settings UI |
| --- | --- | --- | --- | --- |
| **Telegram** | Two-way | Bot API long-poll | Connect via OpenHuman (managed DM) **or** your own BotFather token | Yes |
| **Discord** | Two-way | Gateway | Your own bot token, OAuth install, **or** managed account link | Yes |
| **Web** | Two-way | In-app | Built-in, no setup (local) | Yes |
| **iMessage** | Two-way | macOS Messages (AppleScript) | Local-only, no credentials (needs Full Disk Access) | Yes |
| **Lark / Feishu** | Two-way | WebSocket or webhook | Your own app id + secret | Yes |
| **DingTalk** | Two-way | Stream Mode WebSocket | Your own client id + secret | Yes |
| **元宝 (Yuanbao)** | Two-way | WebSocket | Your own AppID + AppSecret | Yes |
| **Slack** | Two-way | Events/socket | Your own bot token | `config.toml` |
| **WhatsApp** | Two-way | Meta Cloud webhook | Your own access token | `config.toml` |
| **IRC** | Two-way | Persistent socket | Your own server/nick | `config.toml` |
| **Signal** | Two-way | signal-cli REST events | Your own linked signal-cli account | `config.toml` |
| **Mattermost** | Two-way | WebSocket | Your own bot token | `config.toml` |
| **QQ** | Two-way | WebSocket | Your own bot credentials | `config.toml` |
| **Linq** | Two-way (SMS) | Webhook | Your own API token | `config.toml` |
| **Email** | Two-way | IMAP IDLE + SMTP | Your own mailbox credentials | `config.toml` |

WhatsApp also has an experimental peer-to-peer variant behind the `whatsapp-web` feature flag. Channels marked "webhook" keep a live connection alive but receive inbound messages by HTTP push, so they need a reachable HTTPS endpoint configured on the provider's side.

Telegram is the most fully featured channel. It supports typing indicators and live draft updates, and is currently the only channel wired to a per-channel approval surface, so `Prompt`-class tool calls can be answered inline rather than parked. Discord adds native threaded replies; Lark also threads. Web supports rich text and stays entirely local.

**Email deserves a special mention**: it is a fully **native, self-hosted connector**, with no third-party broker in the loop. Inbound mail arrives over IMAP with **IMAP IDLE** push (new mail reaches the agent in seconds, with the connection refreshed every ~29 minutes per the RFC), and replies go out over SMTP with full attachment/multipart support, from your own address on any provider you configure. An `allowed_senders` allowlist is the inbound security gate. Set it explicitly to the addresses you trust. (In `config.toml` an empty list means deny-all, but the Connections UI defaults a blank field to `["*"]`, which allows **any** sender. So don't leave it blank if strangers shouldn't be able to prompt your agent by email.)

***

## Credential modes

Channels authenticate one of a few ways:

* **Connect via OpenHuman (managed)**: a one-click, encrypted connection brokered through the OpenHuman backend. Today this covers Telegram (message the managed bot directly) and Discord (link your account or install via OAuth). No tokens live on your machine.
* **Your own credentials**: you supply a bot token, API key/secret, or app credentials. Telegram (BotFather token), Discord (bot token), Slack, WhatsApp, Lark/Feishu, DingTalk, Yuanbao, Matrix, Signal, Mattermost, QQ, Linq, IRC, and Email all support this. Maximum control; you own the platform account, rate limits, and any webhook endpoint.
* **Local, no credentials**: the **Web** chat and **iMessage** need no tokens at all. Web runs inside the desktop app; iMessage drives the local macOS Messages app over an AppleScript bridge (grant Full Disk Access). Both keep messages on your machine.

Secrets supplied for any mode are stored through OpenHuman's credential layer and protected at rest by the [encryption layer](privacy-and-security.md). They are never written to `config.toml` in plaintext for the UI-managed channels.

***

## Where to connect a channel

Channels are set up under **Connections → Channels** in the left sidebar — **not** under Settings, and not under any "Automation & Channels" menu (no such menu exists). Open that tab, pick a platform tile, and follow its setup card:

* **Discord** — choose *Connect via OpenHuman* (link your account or install the bot via OAuth), or paste your own Discord bot token.
* **Telegram** — message the managed OpenHuman bot to link, or paste a BotFather bot token.

Slack is connected as an **app** under **Connections → OAuth** (Composio) so the agent can read and act in Slack; it is not set up as a talk-back channel in the Channels tab.

***

## Choosing the default channel

Open **Connections → Channels** to pick which channel is the **active route**: the one OpenHuman uses for proactive, recipient-less delivery (cron, triggers, subconscious). The default is the in-app **Web** chat until you change it. Setting a new default takes effect immediately, without restarting the channel runtime, and the panel shows which channel is currently active. Inbound messages always get answered on whatever channel they arrived on, regardless of the default route.

***

## See also

* [Integrations](integrations/README.md): the read-side catalog the agent pulls context from.
* [Triggers](integrations/triggers.md): live events that fire proactive channel delivery.
* [Subconscious Loop](subconscious.md): the background loop that can reach you through the active channel.
* [Privacy & Security](privacy-and-security.md): where credentials live and the backend boundary.
* [OS Keyring & Secret Storage](os-keyring-and-secret-storage.md): at-rest protection for channel secrets.

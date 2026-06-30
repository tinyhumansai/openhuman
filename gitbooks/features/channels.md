---
description: >-
  Messaging platforms OpenHuman talks back to you on — inbound dispatch into the
  agent loop, outbound replies and proactive delivery, and per-channel
  credentials.
icon: messages-square
---

# Messaging Channels

A **channel** is a messaging platform OpenHuman uses to _talk back_ to you. This is the mirror image of an [integration](integrations/README.md): an integration is mostly a source the agent _reads from_ (your inbox, your calendar, your CRM), while a channel is a two-way conversation surface — you message the agent on a platform you already use, and the agent replies there.

Under the hood every channel implements one small Rust contract — a `send` path for outbound messages and a `listen` path for inbound ones — so the same agent loop serves Telegram, Discord, the built-in web chat, and a dozen others without per-platform branching in the core.

***

## What a channel does

Each channel does two things:

* **Inbound** — when a message arrives, the channel normalizes it into a `ChannelMessage` (sender, reply target, content, optional thread id) and hands it to the dispatch loop. Dispatch spawns or resumes an agent run, scopes its tools, and the agent works the request. Some platforms support a `/models` and `/model` command to switch the model for that sender's session; Telegram additionally supports remote-control commands.
* **Outbound** — the agent's response is sent back through the same channel to your `reply_target`, threaded when the platform supports it. Channels can also deliver **proactively** (no incoming message to reply to) when fired by a [trigger](integrations/triggers.md), a cron job, or the [subconscious loop](subconscious.md). A channel only receives proactive sends if it advertises a default delivery target; channels without one are skipped rather than posted to an empty recipient.

Channels that support it can show a typing indicator, stream progressive **draft updates**, post **threaded replies**, and add **emoji reactions** — capabilities are declared per channel, not assumed.

***

## Supported channels

OpenHuman ships **18 channel provider modules** (16 built by default plus two behind Cargo feature flags), of which **17 are real messaging platforms** — `presentation` is an internal response-rendering helper for the web chat, not a platform you connect to. A separate `cli` channel serves the `openhuman-core` terminal binary. Seven channels are exposed in the Settings UI; the rest are enabled through `config.toml`.

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
| **Matrix** | Two-way | Sync loop | Your own homeserver creds (feature `channel-matrix`) | `config.toml` |
| **Signal** | Two-way | signal-cli REST events | Your own linked signal-cli account | `config.toml` |
| **Mattermost** | Two-way | WebSocket | Your own bot token | `config.toml` |
| **QQ** | Two-way | WebSocket | Your own bot credentials | `config.toml` |
| **Linq** | Two-way (SMS) | Webhook | Your own API token | `config.toml` |
| **Email** | Two-way | IMAP IDLE + SMTP | Your own mailbox credentials | `config.toml` |

WhatsApp also has an experimental peer-to-peer variant behind the `whatsapp-web` feature flag. Channels marked "webhook" keep a live connection alive but receive inbound messages by HTTP push, so they need a reachable HTTPS endpoint configured on the provider's side.

Telegram is the most fully featured channel — it supports typing indicators and live draft updates, and is currently the only channel wired to a per-channel approval surface, so `Prompt`-class tool calls can be answered inline rather than parked. Discord adds native threaded replies; Lark also threads. Web supports rich text and stays entirely local.

***

## Credential modes

Channels authenticate one of a few ways:

* **Connect via OpenHuman (managed)** — a one-click, encrypted connection brokered through the OpenHuman backend. Today this covers Telegram (message the managed bot directly) and Discord (link your account or install via OAuth). No tokens live on your machine.
* **Your own credentials** — you supply a bot token, API key/secret, or app credentials. Telegram (BotFather token), Discord (bot token), Slack, WhatsApp, Lark/Feishu, DingTalk, Yuanbao, Matrix, Signal, Mattermost, QQ, Linq, IRC, and Email all support this. Maximum control; you own the platform account, rate limits, and any webhook endpoint.
* **Local, no credentials** — the **Web** chat and **iMessage** need no tokens at all. Web runs inside the desktop app; iMessage drives the local macOS Messages app over an AppleScript bridge (grant Full Disk Access). Both keep messages on your machine.

Secrets supplied for any mode are stored through OpenHuman's credential layer and protected at rest by the [encryption layer](privacy-and-security.md) — never written to `config.toml` in plaintext for the UI-managed channels.

***

## Choosing the default channel

Open **Settings → Automation & Channels → Messaging Channels** to pick which channel is the **active route** — the one OpenHuman uses for proactive, recipient-less delivery (cron, triggers, subconscious). The default is the in-app **Web** chat until you change it. Setting a new default takes effect immediately, without restarting the channel runtime, and the panel shows which channel is currently active. Inbound messages always get answered on whatever channel they arrived on, regardless of the default route.

***

## See also

* [Integrations](integrations/README.md) — the read-side catalog the agent pulls context from.
* [Triggers](integrations/triggers.md) — live events that fire proactive channel delivery.
* [Subconscious Loop](subconscious.md) — the background loop that can reach you through the active channel.
* [Privacy & Security](privacy-and-security.md) — where credentials live and the backend boundary.
* [OS Keyring & Secret Storage](os-keyring-and-secret-storage.md) — at-rest protection for channel secrets.
